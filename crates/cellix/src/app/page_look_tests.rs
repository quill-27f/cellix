//! What a page draws of its own, as the final review of 0.11.0 found it: a rule, a reviewer's
//! note and a paper's folded preamble. Driven through the app and read off the drawn grid and
//! its paint.

use std::fs;
use std::rc::Rc;

use cellix_render::{Grid, Paint};
use tempfile::TempDir;

use super::App;

/// A file opened in a window `width` by 40, on the cell tier or, with `pixels`, in measured type.
fn open(name: &str, text: &str, width: u16, pixels: bool) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (width, 40));
    if pixels {
        app.set_measure(Rc::new(cellix_render::FontMeasure::new(17, 37)));
    }
    app.open(&path).unwrap();
    (dir, app)
}

fn screen(app: &App) -> Grid {
    let mut grid = Grid::new(app.size().0, app.size().1, app.background());
    app.draw(&mut grid);
    grid
}

/// Where a place in the file is drawn: its column, with its fraction, and its row.
fn spot(app: &App, words: &str) -> (f32, u16) {
    let text = app.active_text().unwrap();
    let byte = text.find(words).unwrap_or_else(|| panic!("no {words:?}"));
    let place = text[..byte].chars().count();
    app.editors[app.active()]
        .cell_of(app.focused_area(), place)
        .unwrap_or_else(|| panic!("{words:?} isn't in view"))
}

fn cells(units: i32) -> f32 {
    units as f32 / cellix_render::units(1.0) as f32
}

/// A horizontal rule is one line across the text's column, in a narrow pane and a wide one.
/// In 0.11.0 it was two: the cell tier's dashes drawn as words in the dim colour, over a
/// hairline of the line colour no longer than 60 columns, so it changed colour part way and
/// stopped short of the column's edge in a wide pane (71-sample-changes-row, 82-*).
#[test]
fn a_rule_is_one_line_across_the_text_column() {
    let text = "Words before the rule, set as a paragraph is.\n\n---\n\nWords after it.\n";
    let border = crate::theme::Theme::default().popup_border;
    for width in [90, 150, 240] {
        let (_dir, app) = open("rule.md", text, width, true);
        let grid = screen(&app);
        let (left, before) = spot(&app, "Words before");
        let (_, after) = spot(&app, "Words after");
        let rule = (before + 1..after)
            .find(|y| grid.row_text(*y).contains('─'))
            .expect("the rule's row");
        let lines: Vec<(f32, f32)> = (grid.paint.iter())
            .filter_map(|paint| match paint {
                Paint::Hairline { bounds, color }
                    if *color == border && cells(bounds.y).floor() == f32::from(rule) =>
                {
                    Some((cells(bounds.x), cells(bounds.x + bounds.w)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 1, "one line, at {width}: {lines:?}");
        let end = f32::from(app.editors[0].text_end(app.focused_area()));
        assert!(
            (lines[0].0 - left).abs() < 0.05,
            "from the text's edge: {lines:?}"
        );
        assert!(
            (lines[0].1 - end).abs() < 0.05,
            "to its other: {lines:?} for {end}"
        );
        let dashes = (grid.paint.iter())
            .any(|paint| matches!(paint, Paint::Text { text, .. } if text.contains('─')));
        assert!(!dashes, "no dashes drawn over it, at {width}");
    }
}

/// A reviewer's note in the text (`{>>note<<}`) is shown as a comment is: apart from the
/// words before it, after the comment's ◆ and on the comment's tint. In 0.11.0 it stood 2 px
/// after the period, small and grey, and read as a typo (69 to 72).
#[test]
fn a_reviewers_note_stands_apart_with_the_comment_mark() {
    let text = "The model holds at scale.{>>A note for a reviewer.<<} And then more.\n";
    let theme = crate::theme::Theme::default();
    let (_dir, app) = open("note.md", text, 150, true);
    let grid = screen(&app);
    let (end, row) = spot(&app, "{>>");
    let mark = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Text { x, text, color, .. } if text.starts_with('◆') => {
                Some((cells(*x), *color))
            }
            _ => None,
        })
        .expect("the comment's mark");
    assert_eq!(mark.1, theme.accent, "in the comment's colour");
    let words = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Text { x, text, .. } if text.contains("note for a reviewer") => Some(cells(*x)),
            _ => None,
        })
        .expect("the note's words");
    assert!(mark.0 < words, "the mark goes first");
    let tint = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == theme.tint && cells(bounds.y).floor() == f32::from(row) => {
                Some((cells(bounds.x), cells(bounds.x + bounds.w)))
            }
            _ => None,
        })
        .expect("the comment's tint");
    assert!(
        tint.0 <= mark.0 && tint.1 > words,
        "under the mark and the words"
    );
    // 8 px or more of the page between the period and the tint, at 17 px a column.
    assert!(
        (tint.0 - end) * 17.0 >= 8.0,
        "{} px after the period",
        (tint.0 - end) * 17.0
    );
    // The pill is the tint: the cells under it keep the page's ground, where their own
    // colour stood a row tall and cell-wide behind the pill, a box round a box.
    let (x, y) = spot(&app, "A note");
    assert_eq!(grid.cell(x as u16 + 1, y).style.bg, theme.text.bg);
    // On cells, the mark in the comment's colour, and the note's cells on its tint.
    let (_dir, app) = open("note.md", text, 150, false);
    let grid = screen(&app);
    let (x, y) = spot(&app, "A note");
    let row = grid.row_text(y);
    assert!(row.contains("scale.  ◆ A note for a reviewer."), "{row}");
    let at = row[..row.find('◆').unwrap()].chars().count() as u16;
    assert_eq!(grid.cell(at, y).style.fg, theme.accent);
    assert_eq!(grid.cell(x as u16, y).style.bg, theme.tint);
}

/// A paper's folded preamble counts its lines as everything does: "1 line", which said
/// "1 lines" (109-problem-ask).
#[test]
fn a_one_line_preamble_says_one_line() {
    let text = "\\documentclass{article}\n\\begin{document}\nHello world.\n\\end{document}\n";
    let (_dir, app) = open("short.tex", text, 150, false);
    let shown = screen(&app).text();
    assert!(shown.contains("Preamble, 1 line"), "{shown}");
    assert!(!shown.contains("1 lines"), "{shown}");
    // Measured, the words are paint, which the screen's text has only some letters of.
    let (_dir, app) = open("short.tex", text, 150, true);
    let grid = screen(&app);
    let said = (grid.paint.iter()).find_map(|paint| match paint {
        Paint::Text { text, .. } if text.contains("Preamble") => Some(text.clone()),
        _ => None,
    });
    assert_eq!(said.as_deref(), Some("▸ Preamble, 1 line"));
}

/// A big heading on the page's last row waits to be drawn until it fits, and so does the `#`
/// beside it, which stood in the status bar under the page (s1-page-drag-scroll).
#[test]
fn a_heading_waiting_to_fit_draws_no_mark_under_the_page() {
    let lines: String = (0..60)
        .map(|n| format!("Line {n} of the page.\n\n"))
        .collect();
    let text = format!("{lines}## A heading at the foot\n\nAfter it.\n");
    let (_dir, mut app) = open("foot.md", &text, 150, true);
    let area = app.focused_area();
    let place = text[..text.find("## A heading").unwrap()].chars().count() + 3;
    let editor = &mut app.editors[0];
    editor.selections.set_single(cellix_core::Selection {
        anchor: place,
        head: place,
        goal: None,
    });
    let bottom = editor.text_bounds(area).bottom();
    // Scrolled until the heading is on the page's last row.
    for _ in 0..200 {
        match editor.cell_of(area, place) {
            Some((_, y)) if y == bottom - 1 => break,
            Some((_, y)) if y < bottom - 1 => editor.scroll_by(-1),
            _ => editor.scroll_by(1),
        }
    }
    assert_eq!(
        editor.cell_of(area, place).map(|(_, y)| y),
        Some(bottom - 1)
    );
    let grid = screen(&app);
    let marks: Vec<f32> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Text { y, text, .. } if text == "#" => Some(cells(*y)),
            _ => None,
        })
        .collect();
    assert!(
        marks.iter().all(|y| *y + 1.0 <= f32::from(bottom)),
        "{marks:?} under {bottom}"
    );
    // A few rows up, where it's drawn, so is its mark.
    app.editors[0].scroll_by(4);
    let grid = screen(&app);
    let marked =
        (grid.paint.iter()).any(|paint| matches!(paint, Paint::Text { text, .. } if text == "#"));
    assert!(marked, "the mark beside a heading that's drawn");
}

/// A deletion's spaces at its ends aren't struck. Struck, the space between "baseline" and
/// what a suggestion takes out read as a hyphen joining them, "baseline-by" (69 to 71, 82).
#[test]
fn a_deletions_spaces_at_its_ends_are_not_struck() {
    let text = "The model improves on the baseline{-- by a wide margin --}, and holds.\n";
    let deleted = crate::theme::Theme::default().deleted();
    let (_dir, app) = open("gone.md", text, 150, true);
    let grid = screen(&app);
    let (by, row) = spot(&app, "by a wide");
    let (end, _) = spot(&app, " --}");
    let lines: Vec<(f32, f32)> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Hairline { bounds, color }
                if *color == deleted && cells(bounds.y).floor() == f32::from(row) =>
            {
                Some((cells(bounds.x), cells(bounds.x + bounds.w)))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "one line through it: {lines:?}");
    let (from, to) = lines[0];
    assert!(
        (from - by).abs() < 0.05,
        "from its first letter at {by}: {lines:?}"
    );
    assert!((to - end).abs() < 0.05, "to its last at {end}: {lines:?}");
}

/// An insertion's line runs under its letters, not its spaces at either end, as a deletion's
/// line through it does (0.11.0's release check, P8). Under the space after "clearly" it ran on
/// under the next word, "clearly_improves" (69 to 71), and in a paper 8 px past "these" (82).
#[test]
fn an_insertions_spaces_at_its_ends_are_not_underlined() {
    let text = "The model {++ clearly ++}improves on the baseline, and holds.\n";
    let (_dir, app) = open("added.md", text, 150, true);
    let grid = screen(&app);
    let (from, row) = spot(&app, "clearly");
    let underlined: Vec<(f32, String)> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Text {
                x,
                y,
                text,
                underline: true,
                ..
            } if cells(*y).floor() == f32::from(row) => Some((cells(*x), text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(underlined.len(), 1, "one line under it: {underlined:?}");
    let (x, words) = &underlined[0];
    assert_eq!(words, "clearly", "its letters alone: {underlined:?}");
    assert!(
        (x - from).abs() < 0.05,
        "from its first letter at {from}: {x}"
    );
    // The words beside it are drawn, with no line under them.
    let painted: String = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(painted.contains("improves"), "{painted}");
}

/// A reviewer's note's tint keeps 8 px before its ◆ and after its last letter, and where it
/// wraps it's cut flush, as a browser slices an inline box: it had 6 px before the ◆ and 11
/// after (0.11.0's release review, B7, 69 to 71). Then 8 px on every row put the row it runs
/// on to 8 px into the page's margin, out of the text's column, where nothing else goes
/// (0.12.0's release review, finding 4): that row's tint starts on the column, and the row it
/// runs on from ends at its last letter.
#[test]
fn a_reviewers_notes_tint_keeps_its_air_at_its_ends_and_wraps_flush() {
    let note = "A note for a reviewer that goes on for long enough to run on past the end of \
        the first row it starts on and onto the next one";
    let text = format!("The model holds at scale, and the experiments say so.{{>>{note}<<}}\n");
    let theme = crate::theme::Theme::default();
    let (_dir, app) = open("note.md", &text, 100, true);
    let grid = screen(&app);
    let mut pills: Vec<(f32, f32, f32)> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == theme.tint => Some((
                cells(bounds.y).floor(),
                cells(bounds.x),
                cells(bounds.x + bounds.w),
            )),
            _ => None,
        })
        .collect();
    pills.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(pills.len() >= 2, "it runs on to a second row: {pills:?}");
    // The text's column: where the paragraph's first row starts.
    let column = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Text { x, text, .. } if text.starts_with("The model") => Some(cells(*x)),
            _ => None,
        })
        .expect("the paragraph's first words");
    let count = pills.len();
    for (index, (row, left, right)) in pills.into_iter().enumerate() {
        // What's painted on the pill's row inside it: the mark and the note's words.
        let held: Vec<(f32, f32)> = (grid.paint.iter())
            .filter_map(|paint| match paint {
                Paint::Text {
                    x, y, text, font, ..
                } if cells(*y).floor() == row && cells(*x) > left - 0.05 && cells(*x) < right => {
                    let start = text.chars().take_while(|c| c.is_whitespace()).count();
                    let lead: String = text.chars().take(start).collect();
                    let x = cells(*x) + app.measure.width(&lead, *font);
                    let shown = text.trim();
                    Some((x, x + app.measure.width(shown, *font)))
                }
                _ => None,
            })
            .collect();
        let first = held.iter().map(|h| h.0).fold(f32::MAX, f32::min);
        let last = held.iter().map(|h| h.1).fold(f32::MIN, f32::max);
        let (before, after) = ((first - left) * 17.0, (right - last) * 17.0);
        if index == 0 {
            assert!((before - 8.0).abs() < 1.0, "row {row}: {before} px before");
        } else {
            // Run on to, it starts on the column, as its first word does.
            let out = (column - left) * 17.0;
            assert!(out.abs() < 0.5, "row {row}: {out} px into the margin");
            assert!(before.abs() < 0.5, "row {row}: {before} px before");
        }
        if index + 1 == count {
            assert!((after - 8.0).abs() < 1.0, "row {row}: {after} px after");
        } else {
            assert!(
                after.abs() < 0.5,
                "row {row}: {after} px past its last letter"
            );
        }
    }
}
