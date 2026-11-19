//! Selecting as in any document: a click sets where the selection starts, and a Shift+click
//! elsewhere, however far it has scrolled since, selects everything between, across every kind
//! of block a page has. Driven through the app with the pointer on Markdown, LaTeX, a notebook,
//! the source and a terminal, and read off the selection, the drawn grid and the comment made.

use std::fs;
use std::rc::Rc;

use cellix_input::{Event, Key, KeyEvent, Mods, MouseEvent, MouseKind};
use cellix_render::{Grid, Paint, Rect};
use tempfile::TempDir;

use super::{App, Command};

/// A file opened in a window 150 by 40, on the cell tier or, with `pixels`, in measured type.
fn open(name: &str, text: &str, pixels: bool) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), (150, 40));
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

fn pointer(kind: MouseKind, fine_x: f32, y: u16, shift: bool) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        x: fine_x as u16,
        y,
        fine_x,
        fine_y: f32::from(y) + 0.5,
        mods: Mods {
            shift,
            ..Mods::default()
        },
    })
}

/// Where the file's text has `words`, counting in chars as the selection does.
fn at(app: &App, words: &str) -> usize {
    let text = app.active_text().unwrap();
    let byte = text
        .find(words)
        .unwrap_or_else(|| panic!("no {words:?} in {text}"));
    text[..byte].chars().count()
}

/// The same, just past the end of `words`.
fn after(app: &App, words: &str) -> usize {
    at(app, words) + words.chars().count()
}

/// Where a place in the text is drawn, while it's in view.
fn spot(app: &App, place: usize) -> Option<(f32, u16)> {
    let area = app.focused_area();
    app.editors[app.active()].cell_of(area, place)
}

/// A press and its release on a place in the text, as a terminal reports a click, a little
/// inside the letter there.
fn click(app: &mut App, place: usize, shift: bool) {
    let (x, y) = spot(app, place).unwrap_or_else(|| panic!("{place} isn't in view"));
    app.handle(pointer(MouseKind::Left, x + 0.1, y, shift));
    app.handle(pointer(MouseKind::Up, x + 0.1, y, shift));
}

/// Turns the wheel over the page until a place in the text is in view.
fn scroll_to(app: &mut App, place: usize) {
    let area = app.focused_area();
    let over = f32::from(area.x + area.w / 2);
    let down = place > app.editors[app.active()].selections.primary().head;
    let kind = if down {
        MouseKind::ScrollDown
    } else {
        MouseKind::ScrollUp
    };
    for _ in 0..400 {
        if spot(app, place).is_some() {
            return;
        }
        app.handle(pointer(kind, over, area.y + area.h / 2, false));
    }
    panic!("{place} never came into view");
}

fn selected(app: &App) -> String {
    app.selected_text().unwrap_or_default()
}

/// The file's text from one place to another.
fn source(app: &App, from: usize, to: usize) -> String {
    let text = app.active_text().unwrap();
    text.chars().skip(from).take(to - from).collect()
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(Event::Key(KeyEvent::plain(Key::Char(c))));
    }
}

fn filler(n: usize) -> String {
    (0..n)
        .map(|i| format!("Filler paragraph {i} keeps the end of the page out of view.\n\n"))
        .collect()
}

/// A page with a block of every kind before a long way down to its last paragraph.
fn markdown() -> String {
    let blocks = "# Start of the scan\n\n\
        The first paragraph begins the scan.\n\n\
        ## A heading inside\n\n\
        - an item in a list\n\
        - another item\n\n\
        > a quoted line\n\n\
        | name | value |\n\
        |------|-------|\n\
        | cell | 42 |\n\n\
        ```rust\n\
        let code = 1;\n\
        ```\n\n\
        $$\n\
        x^2 + y^2 = z^2\n\
        $$\n\n";
    format!(
        "{blocks}{}The last paragraph ends the scan here.\n",
        filler(40)
    )
}

/// A click, then the wheel until the start is long out of view, then a Shift+click at the end.
fn scan(app: &mut App) -> (usize, usize) {
    let (start, end) = (at(app, "first paragraph"), after(app, "ends the scan here"));
    click(app, start, false);
    scroll_to(app, end);
    assert!(
        spot(app, start).is_none(),
        "the start has scrolled out of view"
    );
    click(app, end, true);
    (start, end)
}

#[test]
fn a_shift_click_selects_everything_from_the_click_down_a_page() {
    for pixels in [false, true] {
        let (_dir, mut app) = open("scan.md", &markdown(), pixels);
        let (start, end) = scan(&mut app);
        let text = selected(&app);
        assert_eq!(text, source(&app, start, end), "pixels: {pixels}");
        for block in [
            "## A heading inside",
            "- another item",
            "> a quoted line",
            "| cell | 42 |",
            "let code = 1;",
            "x^2 + y^2 = z^2",
            "Filler paragraph 39",
        ] {
            assert!(text.contains(block), "{block:?} in {text:?}");
        }
        if !pixels {
            // And it's drawn as selected, to the last word.
            let grid = screen(&app);
            let (x, y) = spot(&app, at(&app, "Filler paragraph 39")).unwrap();
            let theme = crate::theme::Theme::default();
            assert_eq!(grid.cell(x as u16, y).style.bg, theme.selection);
            let (x, y) = spot(&app, end - 1).unwrap();
            assert_eq!(grid.cell(x as u16, y).style.bg, theme.selection);
        }
    }
}

#[test]
fn a_shift_click_again_moves_the_end_and_keeps_the_start() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    let (start, _) = scan(&mut app);
    let nearer = at(&app, "Filler paragraph 30");
    scroll_to(&mut app, nearer);
    click(&mut app, nearer, true);
    assert_eq!(selected(&app), source(&app, start, nearer));
    // Before where it started, it runs back from there.
    let top = at(&app, "Start of the scan");
    scroll_to(&mut app, top);
    click(&mut app, top, true);
    assert_eq!(selected(&app), source(&app, top, start));
    assert_eq!(app.editors[0].selections.primary().anchor, start);
}

#[test]
fn a_shift_click_extends_a_selection_already_made_from_its_anchor() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    // Dragged over "begins the".
    let (from, to) = (at(&app, "begins"), after(&app, "begins the"));
    let (x, y) = spot(&app, from).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let (x, y) = spot(&app, to).unwrap();
    app.handle(pointer(MouseKind::Drag, x + 0.1, y, false));
    app.handle(pointer(MouseKind::Up, x + 0.1, y, false));
    assert_eq!(selected(&app), "begins the");
    let end = after(&app, "another item");
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, from, end));
    // A word a double click selected keeps its start too.
    let word = at(&app, "quoted");
    let (x, y) = spot(&app, word).unwrap();
    for _ in 0..2 {
        app.handle(pointer(MouseKind::Left, x + 1.1, y, false));
        app.handle(pointer(MouseKind::Up, x + 1.1, y, false));
    }
    assert_eq!(selected(&app), "quoted");
    let end = after(&app, "let code");
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, word, end));
}

#[test]
fn a_shift_click_takes_a_formula_whole_and_ticks_no_box() {
    let text = "Before the task.\n\n- [ ] a task to do\n\nThen a formula:\n\n$$\nx^2 + y^2 = z^2\n$$\n\nAfter it.\n";
    let (_dir, mut app) = open("marks.md", text, false);
    let start = at(&app, "Before");
    click(&mut app, start, false);
    // The box is where the list's marker is: the Shift+click selects up to it and leaves it be.
    let item = at(&app, "a task");
    let (x, y) = spot(&app, item).unwrap();
    let box_x = x - 2.0;
    app.handle(pointer(MouseKind::Left, box_x, y, true));
    app.handle(pointer(MouseKind::Up, box_x, y, true));
    assert_eq!(app.active_text().unwrap(), text, "the box is still empty");
    assert!(
        selected(&app).starts_with("Before the task."),
        "{:?}",
        selected(&app)
    );
    // On a formula, the selection takes all of its source, leaving no caret inside it to open
    // it: in measured type it stays a picture.
    let end = after(&app, "z^2\n$$\n");
    let inside = at(&app, "y^2");
    let (x, y) = spot(&app, inside).unwrap();
    app.handle(pointer(MouseKind::Left, x, y, true));
    app.handle(pointer(MouseKind::Up, x, y, true));
    assert_eq!(selected(&app), source(&app, start, end));
    // A plain click on the box still ticks it.
    let (x, y) = spot(&app, item).unwrap();
    app.handle(pointer(MouseKind::Left, x - 2.0, y, false));
    app.handle(pointer(MouseKind::Up, x - 2.0, y, false));
    assert!(app.active_text().unwrap().contains("- [x] a task"));

    let (_dir, mut app) = open("marks.md", text, true);
    click(&mut app, start, false);
    assert!(!screen(&app).text().contains("x^2"), "a picture");
    assert!(!picture_selected(&screen(&app)), "nothing selected yet");
    let (x, y) = spot(&app, inside).expect("the formula in view");
    app.handle(pointer(MouseKind::Left, x, y, true));
    app.handle(pointer(MouseKind::Up, x, y, true));
    assert_eq!(selected(&app), source(&app, start, end));
    let grid = screen(&app);
    assert!(!grid.text().contains("x^2"), "still a picture");
    // Its ink stands on the selection's colour, as the words' before it do.
    assert!(picture_selected(&grid), "the formula drawn as selected");
}

/// Whether a picture on the page stands in the selection's colour, its own shape of it.
fn picture_selected(grid: &Grid) -> bool {
    let selection = crate::theme::Theme::default().selection;
    let pictures: Vec<cellix_render::Bounds> = (grid.paint.iter())
        .filter_map(|paint| match paint {
            Paint::Image { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .collect();
    // Ink stands on the colour with room round it; a wash lies on the picture's edges.
    let round = |outer: &cellix_render::Bounds| {
        pictures.iter().any(|inner| {
            outer.x < inner.x
                && outer.y < inner.y
                && outer.x + outer.w > inner.x + inner.w
                && outer.y + outer.h > inner.y + inner.h
        })
    };
    grid.paint.iter().any(|paint| match paint {
        Paint::Panel { bounds, fill, .. } => *fill == Some(selection) && round(bounds),
        Paint::Wash { bounds, color, .. } => *color == selection && pictures.contains(bounds),
        _ => false,
    })
}

/// The selection's shapes on the page, in the order they're painted: where in the paint each
/// is, and its left, top, right and foot in columns and rows.
fn selection_shapes(grid: &Grid) -> Vec<(usize, [f32; 4])> {
    let selection = crate::theme::Theme::default().selection;
    shapes(grid, selection)
}

/// The same for every panel filled with `colour`.
fn shapes(grid: &Grid, colour: cellix_render::Rgb) -> Vec<(usize, [f32; 4])> {
    let unit = cellix_render::units(1.0) as f32;
    (grid.paint.iter().enumerate())
        .filter_map(|(index, paint)| match paint {
            Paint::Panel {
                bounds,
                fill: Some(fill),
                ..
            } if *fill == colour => {
                let [x, y, w, h] =
                    [bounds.x, bounds.y, bounds.w, bounds.h].map(|v| v as f32 / unit);
                Some((index, [x, y, x + w, y + h]))
            }
            _ => None,
        })
        .collect()
}

/// The shapes that stand on row `y`, as a row's selection does: from its top to its foot.
fn on_row(shapes: &[(usize, [f32; 4])], y: u16) -> Vec<(usize, [f32; 4])> {
    let top = f32::from(y);
    (shapes.iter().copied())
        .filter(|(_, [_, from, _, to])| *from == top && *to == top + 1.0)
        .collect()
}

/// A paragraph with a look of every kind, set long enough to wrap, then lists and a table.
fn every_look() -> String {
    "# A page\n\n\
     Plain words, **bold**, *italic*, `inline code`, ==highlighted== and then a long run of \
     plain words that goes on well past the end of the first row of the page, so that it wraps \
     onto a second row and onto a third one too, which is where the lines of one paragraph have \
     to touch as they do in a browser when it is all selected at once.\n\n\
     - A bullet\n\
     - Another bullet\n\n\
     1. First\n\
     2. Second\n\n\
     | name | value |\n\
     |------|-------|\n\
     | cell | 42 |\n\n\
     After the table.\n"
        .to_string()
}

/// What the user faulted in 0.11.0's selection (scenes s1-*): a seam at every change of
/// look, inline code and a highlight keeping their own grounds as holes in it, each line a
/// band of its own with a gap under it, the list's markers and numbers left out, and a table
/// selected word by word. Now, as a browser or VS Code's Markdown preview draws one, each
/// line selected is one shape the height of its row, so a paragraph's lines touch; it's
/// painted over inline grounds; a list's marker and number are inside it; and a table's
/// cells taken whole are filled from line to line, under the table's lines.
#[test]
fn a_selection_on_a_page_reads_as_one_shape() {
    let (_dir, mut app) = open("looks.md", &every_look(), true);
    let (start, end) = (at(&app, "Plain words"), after(&app, "| cell | 42"));
    click(&mut app, start, false);
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, start, end));
    let grid = screen(&app);
    let selection = selection_shapes(&grid);
    let theme = crate::theme::Theme::default();

    // One shape on each row of the paragraph, none with a seam, each touching the next.
    let first = spot(&app, start).unwrap().1;
    let last = spot(&app, at(&app, "all selected at once")).unwrap().1;
    assert!(last >= first + 2, "the paragraph wraps onto three rows");
    for y in first..=last {
        let row = on_row(&selection, y);
        assert_eq!(
            row.len(),
            1,
            "one shape a row tall on row {y}: {selection:?}"
        );
    }
    // Inline code's pill and the highlighter are under it, not holes in it.
    let band = on_row(&selection, first)[0];
    for ground in [theme.code_bg, theme.highlighter()] {
        let (index, [left, _, right, _]) = (shapes(&grid, ground).into_iter())
            .find(|(_, [_, top, _, _])| top.floor() == f32::from(first))
            .expect("the ground on the first row");
        assert!(index < band.0, "painted before the selection");
        assert!(left >= band.1[0] && right <= band.1[2], "and inside it");
    }

    // A list's bullet and number are inside it: the shape starts before the item's words.
    for item in ["A bullet", "Another bullet", "First", "Second"] {
        let (x, y) = spot(&app, at(&app, item)).unwrap();
        let row = on_row(&selection, y);
        assert_eq!(row.len(), 1, "{item}: {selection:?}");
        assert!(
            row[0].1[0] <= x - 1.5,
            "{item}'s marker is inside: {:?} for words at {x}",
            row[0].1
        );
    }

    // The table's cells taken whole are filled from line to line, under the table's lines.
    let (x, y) = spot(&app, at(&app, "42")).unwrap();
    let cell = (selection.iter().copied())
        .find(|(_, [left, top, right, _])| *top == f32::from(y) && *left < x && *right > x)
        .expect("the cell with 42 selected");
    let [left, _, right, _] = cell.1;
    assert!(
        left <= x - 0.9,
        "from its line, a padding before 42: {left} for {x}"
    );
    assert!(right - left >= 6.0, "to the next: {left}..{right}");
    let line = (grid.paint.iter().enumerate()).rfind(
        |(_, paint)| matches!(paint, Paint::Hairline { color, .. } if *color == theme.popup_border),
    );
    assert!(
        line.is_some_and(|(index, _)| index > cell.0),
        "the table's lines go over it"
    );

    // On cells, the bullet's cell and a cell's padding are selected too.
    let (_dir, mut app) = open("looks.md", &every_look(), false);
    let (start, end) = (at(&app, "Plain words"), after(&app, "| cell | 42"));
    click(&mut app, start, false);
    click(&mut app, end, true);
    let grid = screen(&app);
    let (x, y) = spot(&app, at(&app, "A bullet")).unwrap();
    let bullet = (0..x as u16)
        .rev()
        .find(|c| grid.cell(*c, y).symbol == "•")
        .expect("a bullet before the words");
    assert_eq!(grid.cell(bullet, y).style.bg, theme.selection, "the bullet");
    let (x, y) = spot(&app, at(&app, "42")).unwrap();
    assert_eq!(
        grid.cell(x as u16 + 3, y).style.bg,
        theme.selection,
        "the cell's room after 42"
    );
}

/// A paper's section number and an equation's number are inside the selection as its words
/// are (0.11.0 left out "1" of "1 Introduction" and "(1)").
#[test]
fn a_papers_numbers_are_inside_its_selection() {
    let text = "\\documentclass{article}\n\\begin{document}\n\
        Words before the section.\n\n\
        \\section{Introduction}\n\n\
        Words of the section.\n\n\
        \\begin{equation}\nx^2 + y^2 = z^2\n\\end{equation}\n\n\
        Words after it.\n\n\\end{document}\n";
    let (_dir, mut app) = open("paper.tex", text, true);
    let (start, end) = (at(&app, "Words before"), after(&app, "Words after it"));
    click(&mut app, start, false);
    click(&mut app, end, true);
    assert_eq!(selected(&app), source(&app, start, end));
    let grid = screen(&app);
    let selection = selection_shapes(&grid);
    // The section's number stands before its title, inside the title's shape.
    let (x, y) = spot(&app, at(&app, "Introduction")).unwrap();
    let title = (selection.iter().copied())
        .find(|(_, [left, top, right, foot])| {
            *top <= f32::from(y) + 0.9 && *foot >= f32::from(y) + 1.0 && *left < x && *right > x
        })
        .unwrap_or_else(|| panic!("the title selected: {selection:?}"));
    assert!(
        title.1[0] <= x - 1.0,
        "with its number: {:?} for {x}",
        title.1
    );
    // The equation's number has the selection's colour behind it.
    let unit = cellix_render::units(1.0) as f32;
    let (column, row) = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Text { x, y, text, .. } if text == "(1)" => {
                Some((*x as f32 / unit, *y as f32 / unit))
            }
            _ => None,
        })
        .expect("the equation's number");
    assert!(
        selection.iter().any(|(_, [left, top, right, foot])| {
            *left <= column + 0.1
                && *right >= column + 1.0
                && *top <= row + 0.5
                && *foot >= row + 0.5
        }),
        "(1) at {column}, {row}: {selection:?}"
    );
    // And in the formula's own band, one across the column, where it had a box of its own
    // far right of the formula's (s1-paper-shift-select).
    let formula = (grid.paint.iter())
        .find_map(|paint| match paint {
            Paint::Image { bounds, .. } => Some(bounds.x as f32 / unit + 0.5),
            _ => None,
        })
        .expect("the formula");
    let holding: Vec<_> = (selection.iter())
        .filter(|(_, [left, top, right, foot])| {
            *left <= column + 0.1
                && *right >= column + 1.0
                && *top <= row + 0.5
                && *foot >= row + 0.5
        })
        .collect();
    assert_eq!(holding.len(), 1, "(1) in one shape: {selection:?}");
    let [left, ..] = holding[0].1;
    assert!(
        left <= formula,
        "the shape that holds (1) holds the formula from {formula}: {holding:?}"
    );
}

/// A cell of 17 by 50 px with words set for a line of 36, as the tour's Airy rows are.
const CELL: (usize, usize) = (17, 50);
const LINE: usize = 36;

/// The same file opened with Airy rows, in a window `size` cells.
fn open_airy(name: &str, text: &str, size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut app = App::new(dir.path().to_path_buf(), size);
    let measure = cellix_render::FontMeasure::new(CELL.0, CELL.1).with_type_row(LINE);
    app.set_measure(Rc::new(measure));
    app.open(&path).unwrap();
    (dir, app)
}

/// How near anything else drawn comes to the buttons by a selection, in pixels, from the
/// screen painted as a terminal shows it: the buttons are taken out of the paint, the rows
/// round them are drawn, and every pixel not the page's ground is measured from the buttons'
/// box, across or down whichever is further.
fn air_round(grid: &Grid, buttons: (Rect, Rect)) -> f32 {
    air_about(grid, buttons).round
}

/// How near anything comes to the buttons, in pixels: round them, as `air_round` says; over
/// them and under them, from what's drawn across their columns; and their box, as painted.
struct Air {
    round: f32,
    over: f32,
    under: f32,
    left: f32,
    foot: f32,
}

fn air_about(grid: &Grid, buttons: (Rect, Rect)) -> Air {
    use cellix_render::{Rasterizer, SoftwareRasterizer};
    let (row, line) = (CELL.1 as f32, LINE as f32);
    let unit = cellix_render::units(1.0) as f32;
    let on_buttons = |bounds: &cellix_render::Bounds| {
        let (x, y) = (bounds.x as f32 / unit, bounds.y as f32 / unit);
        [buttons.0, buttons.1].iter().any(|b| {
            x >= f32::from(b.x) - 0.5
                && x < f32::from(b.right())
                && (y - f32::from(b.y)).abs() < 1.0
        })
    };
    // The buttons' box, as the painter stands it in its row: edged, with no shadow, since
    // 0.11.0's release review (B4).
    let edge = crate::theme::Theme::default().popup_border;
    let chip = |paint: &Paint| match paint {
        Paint::Panel {
            bounds,
            border: Some(border),
            ..
        } => *border == edge && on_buttons(bounds),
        _ => false,
    };
    let mut boxes = (grid.paint.iter()).filter_map(|paint| match paint {
        Paint::Panel { bounds, .. } if chip(paint) => {
            let (top, tall) = cellix_render::stood(bounds.y, bounds.h, row, line);
            let left = bounds.x as f32 / unit * CELL.0 as f32;
            Some([
                left,
                top,
                left + bounds.w as f32 / unit * CELL.0 as f32,
                top + tall,
            ])
        }
        _ => None,
    });
    let first = boxes.next().expect("the buttons are painted");
    let [left, top, right, foot] = boxes.fold(first, |a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    });
    // Everything else, drawn on the rows round them.
    let mut without = grid.clone();
    let labels = ["Comment", "Ask Agent"];
    without.paint.retain(|paint| match paint {
        Paint::Panel { .. } => !chip(paint),
        Paint::Text { text, .. } => !labels.iter().any(|l| text.starts_with(l)),
        _ => true,
    });
    let ground = crate::theme::Theme::default().text.bg;
    let mut raster = SoftwareRasterizer::new(CELL.0, CELL.1)
        .expect("fonts are in the binary")
        .with_type_row(LINE);
    let rows = buttons.0.y.saturating_sub(2)..(buttons.0.y + 3).min(grid.height());
    let mut air = Air {
        round: f32::MAX,
        over: f32::MAX,
        under: f32::MAX,
        left,
        foot,
    };
    for y in rows {
        let image = raster.row(&without, y);
        for py in 0..image.height {
            let down = (y as usize * CELL.1 + py) as f32 + 0.5;
            let dy = (top - down).max(down - foot).max(0.0);
            for px in (left as usize).saturating_sub(16)..(right as usize + 16).min(image.width) {
                let at = (py * image.width + px) * 3;
                let colour =
                    cellix_render::Rgb(image.rgb[at], image.rgb[at + 1], image.rgb[at + 2]);
                if colour == ground {
                    continue;
                }
                let across = px as f32 + 0.5;
                let dx = (left - across).max(across - right).max(0.0);
                air.round = air.round.min(dx.max(dy));
                if dx == 0.0 && down < top {
                    air.over = air.over.min(top - down - 0.5);
                } else if dx == 0.0 && down > foot {
                    air.under = air.under.min(down - 0.5 - foot);
                }
            }
        }
    }
    air
}

/// The air the user asked of everything: 8 px of a Retina screen.
const AIR: f32 = 8.0;

/// One rule for the buttons by a selection (0.11.0's review: scenes s1-*, 73): in a row of
/// their own over the first selected line, 8 px or more clear of words, lines and the
/// selection; over a paragraph's lines, a table or front matter, above them.
#[test]
fn the_selection_bar_stands_over_the_first_line_clear_of_everything() {
    let page = "---\ntitle: Front matter\n---\n\n# A heading of its own\n\n\
        A first paragraph, long enough to run on over two rows of the page and then onto a third \
        one as well, which takes a good many more words than a line holds, since a line of the page \
        holds a hundred letters or more, so that a word on its last row has words over it on the \
        rows before.\n\n\
        | name | value |\n|------|-------|\n| cell | 42 |\n| more | 43 |\n\n\
        A last paragraph.\n";
    let (_dir, mut app) = open_airy("bar.md", page, (150, 40));
    let check = |app: &mut App, word: &str, length: usize| {
        let (from, to) = (at(app, word), at(app, word) + length);
        let (x, y) = spot(app, from).unwrap();
        app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
        let (x2, y2) = spot(app, to).unwrap();
        app.handle(pointer(MouseKind::Drag, x2 + 0.1, y2, false));
        app.handle(pointer(MouseKind::Up, x2 + 0.1, y2, false));
        assert_eq!(selected(app).chars().count(), length, "{word}");
        let bar = app.selection_bar().expect("the buttons");
        let air = air_round(&screen(app), bar);
        assert!(air >= AIR, "{word}: {air} px from the buttons");
        (bar.0.y, y)
    };
    // A word on the paragraph's last row: over the paragraph, not on its words, and in the
    // middle of the gap there, since it's for neither what's over it nor the paragraph's
    // first row (0.11.0's release check, P1).
    let (bar, line) = check(&mut app, "words over it", 5);
    let first = spot(&app, at(&app, "A first paragraph")).unwrap().1;
    assert!(line >= first + 2, "the word is on the third row");
    assert_eq!(bar, first - 1, "in the row over the paragraph");
    let air = air_about(&screen(&app), app.selection_bar().unwrap());
    assert!(
        (air.over - air.under).abs() <= 2.0,
        "in the middle: {} px over, {} under",
        air.over,
        air.under
    );
    // A cell of the table's second line: over the table, clear of its box.
    let (bar, _) = check(&mut app, "43", 2);
    let head = spot(&app, at(&app, "name")).unwrap().1;
    assert_eq!(bar, head - 1, "in the row over the table");
    // A heading's word right under front matter: between the two, clear of both.
    let (bar, line) = check(&mut app, "heading", 7);
    assert!(bar < line, "over the heading");
}

/// With its start scrolled out of view, the bar goes under the last selected line, clear of
/// what's there; with blocks under it to the foot, beside it.
#[test]
fn the_selection_bar_under_or_beside_the_end_keeps_clear_too() {
    let lines: String = (0..90).map(|n| format!("| row {n} | {n} |\n")).collect();
    let page = format!(
        "# Top\n\nThe start of the selection.\n\n{}| name | value |\n|------|-------|\n{lines}",
        filler(30)
    );
    let (_dir, mut app) = open_airy("end.md", &page, (150, 40));
    let start = at(&app, "start of the selection");
    click(&mut app, start, false);
    // Scrolled until the table's tenth row is in view, the start long out of it.
    let end = after(&app, "row 10");
    scroll_to(&mut app, end);
    for _ in 0..3 {
        app.handle(pointer(MouseKind::ScrollDown, 60.0, 20, false));
    }
    assert!(spot(&app, start).is_none(), "the start is out of view");
    let (_, y) = spot(&app, end).expect("the end in view");
    click(&mut app, end, true);
    let bar = app.selection_bar().expect("the buttons");
    assert_eq!(
        bar.0.y, y,
        "beside the last line: the table runs on under it"
    );
    let (x, _) = spot(&app, after(&app, "| row 10 | 10")).unwrap();
    assert!(f32::from(bar.0.x) > x, "after the table");
    let air = air_round(&screen(&app), bar);
    assert!(air >= AIR, "{air} px from the buttons");
}

/// One distance from the buttons to their selection (0.11.0's release check, P1, where it was
/// 5 to 45 px scene by scene): their foot 12 px over its first line where they stand over it,
/// their top 12 px under its last where they stand under it, and in the middle of the gap they
/// stand in where that would leave less on its other side. Their left edge is at the
/// selection's start, or at the text column's with the start out of view.
#[test]
fn the_selection_bar_stands_twelve_pixels_from_its_selection() {
    let mut page = String::from(
        "---\ntitle: Front matter\nauthor: Someone who writes\n---\n\n\
         # Everything a page can do\n\n\
         A paragraph of a few words under the heading.\n\n## Text\n\n\
         Plain words of a paragraph that runs on under a heading of its own.\n\n",
    );
    page.push_str(&filler(30));
    page.push_str(
        "## Tables\n\nRight-click a cell for rows, columns and alignment. Click any cell to type \
         in it, and Tab goes on.\n\n| name | value |\n|------|-------|\n",
    );
    page.push_str(
        &(0..40)
            .map(|n| format!("| row {n} | {n} |\n"))
            .collect::<String>(),
    );
    page.push_str("\nThe last paragraph of the page.\n");
    let (_dir, mut app) = open_airy("gap.md", &page, (150, 40));
    let near = |a: f32, b: f32| (a - b).abs() <= 1.5;
    let drag = |app: &mut App, from: usize, to: usize| {
        let (x, y) = spot(app, from).unwrap();
        app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
        let (x2, y2) = spot(app, to).unwrap();
        app.handle(pointer(MouseKind::Drag, x2 + 0.1, y2, false));
        app.handle(pointer(MouseKind::Up, x2 + 0.1, y2, false));
        assert_eq!(selected(app), source(app, from, to));
    };
    let start_px = |app: &App, place: usize| spot(app, place).unwrap().0 * CELL.0 as f32;

    // A word of a heading under front matter (73, where they stood 9 px under the front
    // matter's box and 30 over the word): over the word, with more air over them.
    for (word, length) in [("page", 4), ("words of", 8)] {
        let from = at(&app, word);
        drag(&mut app, from, from + length);
        let bar = app.selection_bar().expect("the buttons");
        let air = air_about(&screen(&app), bar);
        assert!(bar.0.y < spot(&app, from).unwrap().1, "{word}: over it");
        assert!(near(air.under, 12.0), "{word}: {} px over it", air.under);
        assert!(
            air.over >= 12.0 - 1.5,
            "{word}: {} px under what's over",
            air.over
        );
        let left = start_px(&app, from);
        assert!(
            (air.left - left).abs() < 1.0,
            "{word}: at its start, {left} px, not {}",
            air.left
        );
    }

    // Its start out of view and its end on the paragraph under "Tables", one row with too
    // little room after its words and a table under it: over the end's line, in a gap too
    // small for 12 px and 12 (s1-page-shift-select-scrolled, 5 px under the heading and 16 over
    // the paragraph).
    let start = at(&app, "Plain words");
    click(&mut app, start, false);
    let end = after(&app, "Right-click a cell");
    let table = at(&app, "| row 20 |");
    scroll_to(&mut app, table);
    let (_, last) = spot(&app, end).expect("the end in view");
    assert!(last >= 5, "the heading over it in view");
    assert!(spot(&app, start).is_none(), "the start is out of view");
    click(&mut app, end, true);
    let bar = app.selection_bar().expect("the buttons");
    assert!(bar.0.y < last, "over the end's line");
    let air = air_about(&screen(&app), bar);
    assert!(
        air.over >= 8.0 && air.under >= 8.0,
        "{} and {}",
        air.over,
        air.under
    );
    assert!(
        // Within 2 px: the words over it are reckoned to their tallest letters' reach.
        air.under < 12.0 + 12.0 && (air.over - air.under).abs() <= 2.0,
        "in the middle of the gap: {} px over, {} under",
        air.over,
        air.under
    );
    let column = start_px(&app, at(&app, "Right-click"));
    assert!(
        (air.left - column).abs() < 1.0,
        "on the text column, {column} px, not {}",
        air.left
    );

    // Its start out of view and its end on the page's last line, with the page's end under
    // it: 12 px under it, on the text column (s1-page-drag-scroll, 45 px under and at the
    // caret's column).
    let end = after(&app, "The last paragraph");
    scroll_to(&mut app, end);
    for _ in 0..3 {
        app.handle(pointer(MouseKind::ScrollDown, 60.0, 20, false));
    }
    let (_, last) = spot(&app, end).expect("the end in view");
    click(&mut app, end, true);
    let bar = app.selection_bar().expect("the buttons");
    assert!(bar.0.y > last, "under the end's line");
    let air = air_about(&screen(&app), bar);
    assert!(near(air.over, 12.0), "{} px under it", air.over);
    let column = start_px(&app, at(&app, "The last paragraph"));
    assert!(
        (air.left - column).abs() < 1.0,
        "on the text column, {column} px, not {}",
        air.left
    );
}

/// Under or over, the buttons stand in the gap next to the block the selection's line is in,
/// never one past what comes after it: with a table after the paragraph a selection ends in,
/// they stood under the table, 11 px over the paragraph after it (the tour of this change,
/// s1-page-shift-select-scrolled).
#[test]
fn the_selection_bar_stays_in_the_gap_next_to_its_block() {
    let page = format!(
        "# Top\n\nThe start of the selection.\n\n{}## Tables\n\nRight-click a cell for rows, \
         columns and alignment. Click any cell to type in it, and Tab goes on.\n\n\
         | name | value |\n|------|-------|\n| cell | 42 |\n\n\
         Try: a paragraph after the table, with a row of room before it.\n\n{}",
        filler(30),
        filler(10)
    );
    let (_dir, mut app) = open_airy("gap.md", &page, (150, 40));
    let start = at(&app, "start of the selection");
    click(&mut app, start, false);
    let after_table = at(&app, "Try: a paragraph");
    scroll_to(&mut app, after_table);
    assert!(spot(&app, start).is_none(), "the start is out of view");
    let end = after(&app, "Right-click a cell");
    let (_, last) = spot(&app, end).expect("the end in view");
    click(&mut app, end, true);
    let bar = app.selection_bar().expect("the buttons");
    let (_, table) = spot(&app, at(&app, "name |")).expect("the table in view");
    assert!(
        bar.0.y < last,
        "over the end's line, not under the table: row {}, the end on {last}, the table from {table}",
        bar.0.y
    );
}

/// Lifted toward their selection, the buttons reach into the row by theirs, and a click there
/// on them, where a terminal says where in a row, presses them.
#[test]
fn a_click_on_the_selection_bar_where_its_painted_presses_it() {
    let page = "---\ntitle: Front matter\nauthor: Someone who writes\n---\n\n\
         # Everything a page can do\n\nA paragraph.\n";
    let (_dir, mut app) = open_airy("press.md", page, (150, 40));
    let from = at(&app, "page can");
    let (x, y) = spot(&app, from).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let (x2, _) = spot(&app, from + 4).unwrap();
    app.handle(pointer(MouseKind::Drag, x2 + 0.1, y, false));
    app.handle(pointer(MouseKind::Up, x2 + 0.1, y, false));
    let bar = app.selection_bar().expect("the buttons");
    let air = air_about(&screen(&app), bar);
    // A few pixels over its foot, in whichever row that is.
    let foot = (air.foot - 3.0) / CELL.1 as f32;
    assert!(foot as u16 > bar.0.y, "the foot is in the row under theirs");
    let press = Event::Mouse(MouseEvent {
        kind: MouseKind::Left,
        x: bar.0.x + 2,
        y: foot as u16,
        fine_x: f32::from(bar.0.x + 2),
        fine_y: foot,
        mods: Mods::default(),
    });
    app.handle(press);
    assert!(app.dock.draft.is_some(), "a comment is being written");
}

#[test]
fn a_shift_click_selection_gets_the_comment_bar_and_its_comment_holds_all_of_it() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    scan(&mut app);
    let grid = screen(&app);
    assert!(grid.text().contains("Ask Agent"), "{}", grid.text());
    let (comment, _) = app.selection_bar().expect("the buttons by the selection");
    app.handle(pointer(
        MouseKind::Left,
        f32::from(comment.x + 1),
        comment.y,
        false,
    ));
    app.handle(pointer(
        MouseKind::Up,
        f32::from(comment.x + 1),
        comment.y,
        false,
    ));
    typed(&mut app, "Why so long");
    app.handle(Event::Key(KeyEvent::plain(Key::Enter)));
    let thread = &app.threads[0];
    assert_eq!(thread.text, "Why so long");
    assert_eq!(thread.line, 3, "on the line it starts on");
    // The quote is all of what was selected, which the page marks and an agent hears.
    assert!(
        thread.quote.starts_with("first paragraph begins the scan.")
            && thread.quote.ends_with("ends the scan here"),
        "{}",
        thread.quote
    );
    for words in ["| cell | 42 |", "x^2 + y^2 = z^2", "ends the scan here"] {
        assert!(
            thread.selection.contains(words),
            "{words:?}: {}",
            thread.selection
        );
        assert!(thread.message().contains(words), "{}", thread.message());
    }
}

#[test]
fn a_shift_click_selects_across_a_paper() {
    let blocks = "\\documentclass{article}\n\\begin{document}\n\
        \\section{Start of the scan}\n\n\
        The first paragraph begins the scan.\n\n\
        \\subsection{A heading inside}\n\n\
        \\begin{itemize}\n\\item an item in a list\n\\item another item\n\\end{itemize}\n\n\
        \\begin{quote}\na quoted line\n\\end{quote}\n\n\
        \\begin{tabular}{ll}\nname & value \\\\\ncell & 42 \\\\\n\\end{tabular}\n\n\
        \\begin{verbatim}\nlet code = 1;\n\\end{verbatim}\n\n\
        \\begin{equation}\nx^2 + y^2 = z^2\n\\end{equation}\n\n";
    let text = format!(
        "{blocks}{}The last paragraph ends the scan here.\n\n\\end{{document}}\n",
        filler(40)
    );
    for pixels in [false, true] {
        let (_dir, mut app) = open("paper.tex", &text, pixels);
        let (start, end) = scan(&mut app);
        let got = selected(&app);
        assert_eq!(got, source(&app, start, end), "pixels: {pixels}");
        for block in [
            "\\subsection{A heading inside}",
            "\\item another item",
            "a quoted line",
            "cell & 42",
            "let code = 1;",
            "x^2 + y^2 = z^2",
        ] {
            assert!(got.contains(block), "{block:?} in {got:?}");
        }
        let grid = screen(&app);
        assert!(grid.text().contains("Ask Agent"));
        // The words selected are counted as the paper's are, so never more than it has.
        let foot = grid.row_text(grid.height() - 1);
        let counts: Vec<usize> = foot
            .split(" words")
            .next()
            .unwrap()
            .split(" of ")
            .map(|n| n.split_whitespace().last().unwrap().parse().unwrap())
            .collect();
        assert!(counts.len() == 2 && counts[0] < counts[1], "{foot}");
    }
}

#[test]
fn a_shift_click_selects_across_a_notebooks_cells() {
    let notebook = r##"{
 "cells": [
  {"cell_type": "markdown", "metadata": {}, "source": ["# Training run\n", "\n", "Some words first.\n"]},
  {"cell_type": "code", "execution_count": 1, "metadata": {}, "outputs": [],
   "source": ["import torch\n", "train()"]},
  {"cell_type": "markdown", "metadata": {}, "source": ["Words at the end.\n"]}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4, "nbformat_minor": 5
}"##;
    let (_dir, mut app) = open("run.ipynb", notebook, false);
    let (start, end) = (at(&app, "words first"), after(&app, "Words at the end"));
    click(&mut app, start, false);
    click(&mut app, end, true);
    let got = selected(&app);
    assert_eq!(got, source(&app, start, end));
    assert!(
        got.contains("import torch") && got.contains("train()"),
        "{got}"
    );
}

#[test]
fn a_shift_click_in_the_source_selects_the_same_way() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    app.run(Command::MarkdownPreview);
    assert!(
        screen(&app).text().contains("# Start of the scan"),
        "the source"
    );
    let (start, end) = scan(&mut app);
    assert_eq!(selected(&app), source(&app, start, end));
    assert!(screen(&app).text().contains("Ask Agent"));
}

/// Advances the clock the loop would, a twentieth of a second at a time, until `done`.
fn hold(app: &mut App, mut done: impl FnMut(&App) -> bool) {
    let mut now = std::time::Instant::now();
    for _ in 0..400 {
        if done(app) {
            return;
        }
        now += std::time::Duration::from_millis(50);
        app.step_autoscroll(now);
    }
    panic!("it never got there");
}

#[test]
fn a_drag_held_past_the_foot_scrolls_the_page_and_the_selection_follows() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    let start = at(&app, "first paragraph");
    let (x, y) = spot(&app, start).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let last = at(&app, "ends the scan here");
    assert!(spot(&app, last).is_none(), "the end is out of view");
    // Onto the bar at the foot, under the page, and held there.
    let foot = app.size().1 - 1;
    app.handle(pointer(MouseKind::Drag, 60.0, foot, false));
    assert!(
        app.autoscroll.is_some() && app.soon(),
        "timed while it's held there"
    );
    let top = app.editors[0].top();
    hold(&mut app, |app| {
        let head = app.editors[0].selections.primary().head;
        assert!(
            spot(app, head).is_some(),
            "the end stays in view as it scrolls"
        );
        spot(app, last).is_some()
    });
    assert!(app.editors[0].top() > top, "the page scrolled");
    let area = app.focused_area();
    let edge = app.editors[0].scroll_region(area).bottom() - 1;
    let under = app.editors[0].position_at(area, 60.0, edge);
    let primary = app.editors[0].selections.primary();
    assert_eq!((primary.anchor, primary.head), (start, under));
    assert!(
        selected(&app).contains("Filler paragraph 39"),
        "{}",
        selected(&app)
    );
    // Back up past the top, over the tabs, it scrolls the other way.
    let top = app.editors[0].top();
    app.handle(pointer(MouseKind::Drag, 60.0, 1, false));
    hold(&mut app, |app| app.editors[0].top() + 10 < top);
    assert_eq!(app.editors[0].selections.primary().anchor, start);
    // Back inside it stops, and let go nothing is timed.
    app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
    assert!(app.autoscroll.is_none());
    app.handle(pointer(MouseKind::Drag, 60.0, foot, false));
    app.handle(pointer(MouseKind::Up, 60.0, foot, false));
    assert!(app.autoscroll.is_none(), "nothing left timed");
    assert!(
        screen(&app).text().contains("Ask Agent"),
        "the bar, as after any drag"
    );
}

/// The sample page the tour opens, in the tour's window: Airy rows, 150 by 45.
fn sample_page() -> (TempDir, App) {
    let sample = include_str!("../../../../samples/writing.md");
    open_airy("writing.md", sample, (150, 45))
}

/// Scene 73: a double click on a word of the sample's title, under its front matter, selects
/// that word and brings the buttons. 0.11.0's tour once pictured a caret there instead, which
/// was the tour's two presses arriving apart under load, not the page.
#[test]
fn a_double_click_selects_a_word_of_the_samples_title() {
    let (_dir, mut app) = sample_page();
    let word = at(&app, "# Everything a page") + "# Everything a ".len();
    let (x, y) = spot(&app, word).expect("the title is in view");
    for _ in 0..2 {
        app.handle(pointer(MouseKind::Left, x + 1.0, y, false));
        app.handle(pointer(MouseKind::Up, x + 1.0, y, false));
    }
    assert_eq!(selected(&app), "page");
    let text = screen(&app).text();
    assert!(text.contains("Ask Agent"), "{text}");
}

/// Scene s1-page-drag-scroll: a drag from the paragraph under "Text", held on the status bar's
/// words under the page, scrolls the sample down until let go, the selection going with it.
#[test]
fn a_drag_held_on_the_status_bar_scrolls_the_sample_page() {
    let (_dir, mut app) = sample_page();
    let start = at(&app, "Plain words");
    let (x, y) = spot(&app, start).expect("the paragraph is in view");
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    let tables = at(&app, "## Tables");
    assert!(spot(&app, tables).is_none(), "Tables starts out of view");
    let grid = screen(&app);
    let foot = (0..grid.height())
        .rev()
        .find(|&y| grid.row_text(y).contains("UTF-8"))
        .expect("the status bar says the encoding");
    app.handle(pointer(MouseKind::Drag, 90.0, foot, false));
    assert!(app.autoscroll.is_some(), "held past the page's foot");
    hold(&mut app, |app| spot(app, tables).is_some());
    app.handle(pointer(MouseKind::Up, 90.0, foot, false));
    assert!(app.autoscroll.is_none(), "let go, nothing's timed");
    let picked = selected(&app);
    assert!(
        picked.starts_with("Plain words") && picked.contains("Callouts"),
        "{picked}"
    );
    assert!(screen(&app).text().contains("Ask Agent"));
}

#[test]
fn turning_the_wheel_while_dragging_scrolls_and_the_selection_follows() {
    for source_view in [false, true] {
        let (_dir, mut app) = open("scan.md", &markdown(), false);
        if source_view {
            app.run(Command::MarkdownPreview);
        }
        let start = at(&app, "first paragraph");
        let (x, y) = spot(&app, start).unwrap();
        app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
        app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
        let top = app.editors[0].top();
        for _ in 0..8 {
            app.handle(pointer(MouseKind::ScrollDown, 60.0, 20, false));
        }
        assert!(app.editors[0].top() > top, "scrolled");
        let area = app.focused_area();
        let under = app.editors[0].position_at(area, 60.0, 20);
        let primary = app.editors[0].selections.primary();
        assert_eq!(
            (primary.anchor, primary.head),
            (start, under),
            "source: {source_view}"
        );
        assert!(app.autoscroll.is_none(), "inside, nothing's timed");
        app.handle(pointer(MouseKind::Up, 60.0, 20, false));
        assert_eq!(selected(&app), source(&app, start, under));
    }
}

#[test]
fn nothing_is_timed_unless_a_drag_is_held_past_an_edge() {
    let (_dir, mut app) = open("scan.md", &markdown(), false);
    assert!(app.autoscroll.is_none());
    let start = at(&app, "first paragraph");
    let (x, y) = spot(&app, start).unwrap();
    app.handle(pointer(MouseKind::Left, x + 0.1, y, false));
    app.handle(pointer(MouseKind::Drag, 60.0, 20, false));
    app.handle(pointer(MouseKind::Up, 60.0, 20, false));
    assert!(app.autoscroll.is_none(), "a drag inside the page");
    // Over the bar at the foot with no drag, nothing.
    let foot = app.size().1 - 1;
    app.handle(pointer(MouseKind::Move, 60.0, foot, false));
    assert!(app.autoscroll.is_none());
    assert!(
        !app.step_autoscroll(std::time::Instant::now()),
        "nothing to step"
    );
}

#[cfg(unix)]
mod terminal {
    use super::*;
    use std::time::{Duration, Instant};

    /// A program in the side pane's terminal, once it has printed `ready`.
    fn run(app: &mut App, script: &str) {
        let argv = ["sh".to_string(), "-c".to_string(), script.to_string()];
        let root = app.workspace.root().to_path_buf();
        app.start_session("Stand-in", &argv, &[], &root);
        let deadline = Instant::now() + Duration::from_secs(120);
        while !screen(app).text().contains("ready") {
            assert!(Instant::now() < deadline, "it never started");
            app.poll();
            app.fit_dock();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Where `words` are in the terminal, on the screen.
    fn find(app: &App, words: &str) -> Option<(u16, u16)> {
        let pane = app.dock_screen().unwrap();
        let grid = screen(app);
        (pane.y..pane.bottom()).find_map(|y| {
            let row: String = (grid.row_text(y).chars())
                .skip(usize::from(pane.x))
                .take(usize::from(pane.w))
                .collect();
            let at = row.find(words)?;
            Some((pane.x + row[..at].chars().count() as u16, y))
        })
    }

    fn press(app: &mut App, kind: MouseKind, (x, y): (u16, u16), shift: bool) {
        app.handle(pointer(kind, f32::from(x), y, shift));
    }

    fn wheel_to(app: &mut App, words: &str, up: bool) -> (u16, u16) {
        let pane = app.dock_screen().unwrap();
        let over = (pane.x + pane.w / 2, pane.y + pane.h / 2);
        let kind = if up {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        };
        for _ in 0..400 {
            if let Some(found) = find(app, words) {
                return found;
            }
            press(app, kind, over, false);
        }
        panic!("{words:?} never showed");
    }

    /// Where the buttons by output selected in a terminal stand, their box's foot in pixels
    /// from the window's top, as it's painted on rows of 50 with a line of 36.
    fn output_bar_foot(app: &App) -> f32 {
        let (comment, ask) = app.output_bar().expect("the buttons by the output");
        let edge = crate::theme::Theme::default().popup_border;
        let unit = cellix_render::units(1.0);
        let foot = screen(app).paint.iter().find_map(|paint| match paint {
            Paint::Panel {
                bounds,
                border: Some(border),
                ..
            } if *border == edge
                && (bounds.y / unit - i32::from(comment.y)).abs() <= 1
                && bounds.x / unit >= i32::from(comment.x) - 1
                && bounds.x / unit < i32::from(ask.right()) =>
            {
                let (top, tall) = cellix_render::stood(bounds.y, bounds.h, 50.0, 36.0);
                Some(top + tall)
            }
            _ => None,
        });
        foot.expect("their box is painted")
    }

    /// The buttons by output selected in a terminal stand as a page's do: their foot 12 px
    /// over the selection with clear rows over it, and 12 px over it still where no row is
    /// clear and they stand over the end of the output before it, where they stood 6 px over
    /// it (0.11.0's release review of track P, 108-terminal-ask).
    #[test]
    fn the_buttons_by_selected_output_stand_twelve_pixels_over_it() {
        let (_dir, mut app) = open_airy("a.md", "# A page\n", (160, 40));
        app.run(Command::ListAgents);
        let width = usize::from(app.dock_screen().unwrap().w) - 2;
        let full = |c: &str| c.repeat(width);
        let script = format!(
            "printf 'ready\\n\\n\\n\\nThe line to ask about\\n{}\\n{}\\n{}\\n'; sleep 30",
            full("a"),
            full("c"),
            full("b")
        );
        run(&mut app, &script);
        // Clear rows over it, and then rows of output all round it.
        for words in ["The line to ask about", "ccccccccc"] {
            let (x, y) = find(&app, words).expect("the output");
            let end = (x + 8, y);
            press(&mut app, MouseKind::Left, (x, y), false);
            press(&mut app, MouseKind::Drag, end, false);
            press(&mut app, MouseKind::Up, end, false);
            let (comment, _) = app.output_bar().expect("the buttons by the output");
            assert_eq!(comment.y, y - 1, "{words}: on the row over it");
            let gap = f32::from(y) * 50.0 - output_bar_foot(&app);
            assert!((gap - 12.0).abs() <= 1.0, "{words}: {gap} px over it");
        }
    }

    #[test]
    fn a_shift_click_in_a_terminal_selects_from_the_click_however_far_it_scrolled() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        let lines = "i=1; while [ $i -le 120 ]; do echo \"out $i.\"; i=$((i+1)); done";
        run(&mut app, &format!("{lines}; echo ready; sleep 30"));
        // A click at the end of a line near the foot, and a Shift+click far up the scrollback.
        let (x, y) = wheel_to(&mut app, "out 118.", false);
        let end = (x + "out 118.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        press(&mut app, MouseKind::Up, end, false);
        let start = wheel_to(&mut app, "out 7.", true);
        assert!(find(&app, "out 118.").is_none(), "the click is out of view");
        press(&mut app, MouseKind::Left, start, true);
        press(&mut app, MouseKind::Up, start, true);
        let want: Vec<String> = (7..=118).map(|i| format!("out {i}.")).collect();
        assert_eq!(app.clipboard, want.join("\n"), "copied as a terminal does");
        // Again, nearer: the end moves and the start stays.
        let nearer = wheel_to(&mut app, "out 20.", false);
        press(&mut app, MouseKind::Left, nearer, true);
        press(&mut app, MouseKind::Up, nearer, true);
        let want: Vec<String> = (20..=118).map(|i| format!("out {i}.")).collect();
        assert_eq!(app.clipboard, want.join("\n"));
        // Scrolled, what's selected keeps to its words.
        let session = app.dock.session().unwrap();
        assert_eq!(session.selection().unwrap().0, want.join("\n"));
    }

    #[test]
    fn a_drag_held_past_a_terminals_top_scrolls_its_scrollback_and_the_wheel_does_too() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        let lines = "i=1; while [ $i -le 200 ]; do echo \"out $i.\"; i=$((i+1)); done";
        run(&mut app, &format!("{lines}; echo ready; sleep 30"));
        let (x, y) = find(&app, "out 199.").unwrap();
        let end = (x + "out 199.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        // Over the pane's tabs, above its screen, and held there.
        let pane = app.dock_screen().unwrap();
        let above = (pane.x + 4, pane.y - 1);
        press(&mut app, MouseKind::Drag, above, false);
        assert!(app.autoscroll.is_some());
        super::hold(&mut app, |app| find(app, "out 100.").is_some());
        press(&mut app, MouseKind::Up, above, false);
        assert!(app.autoscroll.is_none());
        let copied = app.clipboard.clone();
        let first = copied.lines().next().unwrap_or_default().to_string();
        let n: usize = first
            .trim_start_matches("out ")
            .trim_end_matches('.')
            .parse()
            .unwrap_or_else(|_| panic!("{copied}"));
        assert!(n <= 100, "from what came into view: {first}");
        assert!(
            copied.ends_with("out 199."),
            "to where it started: {copied}"
        );
        // Turned with the button held, the scrollback moves and the selection follows.
        let (x, y) = find(&app, "out 120.").unwrap();
        let end = (x + "out 120.".len() as u16 - 1, y);
        press(&mut app, MouseKind::Left, end, false);
        let middle = (pane.x, pane.y + pane.h / 2);
        press(&mut app, MouseKind::Drag, middle, false);
        for _ in 0..10 {
            press(&mut app, MouseKind::ScrollUp, middle, false);
        }
        press(&mut app, MouseKind::Up, middle, false);
        let first = app.clipboard.lines().next().unwrap_or_default().to_string();
        let n: usize = first
            .trim_start_matches("out ")
            .trim_end_matches('.')
            .parse()
            .unwrap_or_else(|_| panic!("{}", app.clipboard));
        assert!(n < 100, "{first}");
        assert!(app.clipboard.ends_with("out 120."), "{}", app.clipboard);
    }

    #[test]
    fn shift_selects_in_a_terminal_whose_program_takes_the_mouse() {
        let (_dir, mut app) = open("a.md", "# A page\n", false);
        // Mouse reporting on, as a program that uses the mouse asks.
        run(&mut app, "printf '\\033[?1000h'; echo ready; sleep 30");
        let (x, y) = find(&app, "ready").unwrap();
        press(&mut app, MouseKind::Left, (x, y), false);
        press(&mut app, MouseKind::Drag, (x + 4, y), false);
        press(&mut app, MouseKind::Up, (x + 4, y), false);
        assert_eq!(app.clipboard, "", "the program had that press");
        press(&mut app, MouseKind::Left, (x, y), true);
        press(&mut app, MouseKind::Drag, (x + 4, y), true);
        press(&mut app, MouseKind::Up, (x + 4, y), true);
        assert_eq!(app.clipboard, "ready");
    }
}
