//! The bar above a document: everything a page can do, one click away, in the order people
//! reach for it. Buttons that don't fit move into a menu at its end, so a page beside an
//! agent keeps the same tools.

use cellix_input::Format;
use cellix_render::{Bounds, Font, Grid, Icon, Paint, Rect, Rgb, Style, units};

use crate::rendered::Dialect;
use crate::theme::Theme;

/// A button's width, with air either side of its mark.
const WIDTH: u16 = 3;
/// The space between groups of buttons.
const GAP: u16 = 1;
/// How wide the paragraph style's button is.
const STYLES_WIDTH: u16 = 12;
/// Icons are drawn at this share of a row.
const ICON_SIZE: f32 = 0.72;
const MARK_SIZE: u8 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Styles,
    Format(Format),
    Table,
    Image,
    Diagram,
    Math,
    Comment,
    Track,
    RunCell,
    RunAll,
    Interrupt,
    Restart,
    AddCode,
    AddText,
    Export,
    More,
    Cite,
    Build,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Mark {
        text: &'static str,
        bold: bool,
        italic: bool,
        struck: bool,
    },
    Icon(Icon),
}

struct Button {
    tool: Tool,
    name: &'static str,
    face: Face,
    /// Starts a new group.
    gap: bool,
}

const fn mark(tool: Tool, name: &'static str, text: &'static str, gap: bool) -> Button {
    Button {
        tool,
        name,
        face: Face::Mark {
            text,
            bold: true,
            italic: false,
            struck: false,
        },
        gap,
    }
}

const fn icon(tool: Tool, name: &'static str, icon: Icon, gap: bool) -> Button {
    Button {
        tool,
        name,
        face: Face::Icon(icon),
        gap,
    }
}

const BUTTONS: [Button; 16] = [
    mark(Tool::Format(Format::Bold), "Bold", "B", false),
    Button {
        tool: Tool::Format(Format::Italic),
        name: "Italic",
        face: Face::Mark {
            text: "I",
            bold: false,
            italic: true,
            struck: false,
        },
        gap: false,
    },
    Button {
        tool: Tool::Format(Format::Strikethrough),
        name: "Strikethrough",
        face: Face::Mark {
            text: "S",
            bold: false,
            italic: false,
            struck: true,
        },
        gap: false,
    },
    icon(Tool::Format(Format::Code), "Code", Icon::Code, false),
    icon(Tool::Format(Format::Link), "Link", Icon::Link, false),
    icon(
        Tool::Format(Format::Bullet),
        "Bullet List",
        Icon::ListBullet,
        true,
    ),
    icon(
        Tool::Format(Format::Numbered),
        "Numbered List",
        Icon::ListNumber,
        false,
    ),
    icon(
        Tool::Format(Format::Task),
        "Task List",
        Icon::ListTask,
        false,
    ),
    icon(Tool::Format(Format::Quote), "Quote", Icon::Quote, false),
    icon(Tool::Math, "Formula", Icon::Sigma, true),
    icon(Tool::Table, "Table", Icon::Table, false),
    icon(Tool::Image, "Image", Icon::Picture, false),
    icon(Tool::Diagram, "Diagram", Icon::Diagram, false),
    icon(Tool::Format(Format::Rule), "Divider", Icon::Rule, false),
    icon(Tool::Comment, "Comment", Icon::Comment, true),
    icon(Tool::Track, "Track Changes", Icon::Track, false),
];

/// A notebook's bar: running comes first, then adding cells, then what a text cell needs.
const NOTEBOOK_BUTTONS: [Button; 11] = [
    icon(Tool::RunCell, "Run Cell", Icon::Play, false),
    icon(Tool::RunAll, "Run All Cells", Icon::PlayAll, false),
    icon(Tool::Interrupt, "Interrupt Kernel", Icon::Stop, false),
    icon(Tool::Restart, "Restart Kernel", Icon::Restart, false),
    icon(Tool::AddCode, "Add Code Cell", Icon::AddCell, true),
    icon(Tool::AddText, "Add Text Cell", Icon::AddText, false),
    mark(Tool::Format(Format::Bold), "Bold", "B", true),
    icon(Tool::Format(Format::Link), "Link", Icon::Link, false),
    icon(Tool::Math, "Formula", Icon::Sigma, false),
    icon(Tool::Table, "Table", Icon::Table, false),
    icon(Tool::Comment, "Comment", Icon::Comment, true),
];

const EXPORT: Button = icon(Tool::Export, "Export", Icon::Export, false);
const CITE: Button = icon(Tool::Cite, "Cite", Icon::Cite, false);
const BUILD: Button = icon(Tool::Build, "Build PDF", Icon::Play, false);
const MORE: Button = icon(Tool::More, "More", Icon::More, false);

/// A button on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub tool: Tool,
    pub name: &'static str,
    pub x: u16,
    pub width: u16,
}

fn placed(button: &Button, x: u16) -> Placed {
    Placed {
        tool: button.tool,
        name: button.name,
        x,
        width: WIDTH,
    }
}

/// Which bar a document gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Page(Dialect),
    Notebook,
}

/// The tools for a kind of document. A paper cites where a note has tasks.
fn buttons(bar: Bar) -> impl Iterator<Item = &'static Button> {
    let list: &'static [Button] = if bar == Bar::Notebook {
        &NOTEBOOK_BUTTONS
    } else {
        &BUTTONS
    };
    list.iter().map(move |button| {
        if bar == Bar::Page(Dialect::Latex) && button.tool == Tool::Format(Format::Task) {
            &CITE
        } else {
            button
        }
    })
}

/// The buttons that fit in `row`, styles first and Export at the far end.
pub fn layout(row: Rect, bar: Bar) -> Vec<Placed> {
    let mut out = vec![Placed {
        tool: Tool::Styles,
        name: "Paragraph Style",
        x: row.x + 1,
        width: STYLES_WIDTH,
    }];
    let export = row.right().saturating_sub(WIDTH + 1);
    let build = bar == Bar::Page(Dialect::Latex);
    let edge = export.saturating_sub(if build { WIDTH } else { 0 });
    let mut end = row.x + 1 + STYLES_WIDTH + GAP;
    let last = edge.saturating_sub(WIDTH + GAP);
    let mut hidden = false;
    for button in buttons(bar) {
        let x = end + if button.gap { GAP } else { 0 };
        if hidden || x + WIDTH > last {
            hidden = true;
            continue;
        }
        out.push(placed(button, x));
        end = x + WIDTH;
    }
    // One button left over fits where the menu would go, and needs no menu.
    let left: Vec<&Button> = buttons(bar)
        .filter(|b| !out.iter().any(|p| p.tool == b.tool))
        .collect();
    match left.as_slice() {
        [] => {}
        [one] => out.push(placed(one, end + GAP)),
        _ => out.push(placed(&MORE, end + GAP)),
    }
    if build {
        out.push(placed(&BUILD, edge));
    }
    out.push(placed(&EXPORT, export));
    out
}

/// The buttons that didn't fit, for the menu at the end.
pub fn overflow(row: Rect, bar: Bar) -> Vec<(&'static str, Tool)> {
    let shown = layout(row, bar);
    buttons(bar)
        .filter(|b| !shown.iter().any(|p| p.tool == b.tool))
        .map(|b| (b.name, b.tool))
        .collect()
}

pub fn hit(row: Rect, x: u16, bar: Bar) -> Option<Placed> {
    layout(row, bar)
        .into_iter()
        .find(|p| x >= p.x && x < p.x + p.width)
}

/// How wide the bar must be for all its buttons, since narrower chrome holds no toolbar.
pub const MIN_WIDTH: u16 = STYLES_WIDTH + WIDTH * 3;

fn draw_face(grid: &mut Grid, face: Face, cell: Rect, color: Rgb, style: Style) {
    let (y, middle) = (cell.y, cell.x as f32 + cell.w as f32 / 2.0);
    match face {
        Face::Icon(icon) => {
            let side = ICON_SIZE;
            // A square in pixels: a cell is about twice as tall as it's wide.
            let across = side * 2.0;
            let left = middle - across / 2.0;
            let top = y as f32 + (1.0 - side) / 2.0;
            grid.paint.push(Paint::Icon {
                bounds: Bounds::cells(left, top, across, side),
                icon,
                color,
            });
            // The cell tier shows the name's first letter.
            let letter = fallback(icon);
            let shown = Style {
                fg: color,
                painted: true,
                ..style
            };
            grid.put_str(cell.x + 1, y, letter, shown, cell.right());
        }
        Face::Mark {
            text,
            bold,
            italic,
            struck,
        } => {
            let font = Font {
                bold,
                italic,
                size: MARK_SIZE,
                ..Font::default()
            };
            let shown = Style {
                fg: color,
                bold,
                italic,
                ..style
            };
            grid.paint.push(Paint::Text {
                x: units(middle),
                y: units(y as f32),
                max_width: units(cell.w as f32),
                text: text.to_string(),
                color,
                font,
                align_right: false,
                centred: true,
                underline: false,
            });
            let mut buffer = shown;
            buffer.painted = true;
            grid.put_str(cell.x + 1, y, text, buffer, cell.right());
            if struck {
                let line = Bounds::cells(middle - 0.6, y as f32 + 0.54, 1.2, 0.0);
                grid.hairline(line, color);
            }
        }
    }
}

fn fallback(icon: Icon) -> &'static str {
    match icon {
        Icon::Code => "`",
        Icon::Link => "@",
        Icon::ListBullet => "•",
        Icon::ListNumber => "1",
        Icon::ListTask => "☐",
        Icon::Quote => "▌",
        Icon::Sigma => "Σ",
        Icon::Table => "▦",
        Icon::Picture => "▣",
        Icon::Rule => "─",
        Icon::Comment => "◆",
        Icon::Export => "↓",
        Icon::More => "…",
        Icon::Cite => "[",
        Icon::Play => "▶",
        Icon::Terminal => ">",
        Icon::Diff => "±",
        Icon::Replace => "⇄",
        Icon::ArrowUp => "↑",
        Icon::ArrowDown => "↓",
        Icon::Maximize => "⤢",
        Icon::Minimize => "⤡",
        Icon::Close => "×",
        Icon::ReplaceAll => "⇉",
        Icon::ChevronRight => "›",
        Icon::ChevronDown => "⌄",
        Icon::Globe => "◍",
        Icon::Agents => "◎",
        Icon::Panel => "▭",
        _ => "?",
    }
}

/// One icon in the middle of `cell`, `side` rows tall.
pub fn draw_icon(grid: &mut Grid, (icon, side): (Icon, f32), cell: Rect, color: Rgb, style: Style) {
    let middle = cell.x as f32 + cell.w as f32 / 2.0;
    // A square in pixels: a cell is about twice as tall as it's wide.
    let across = side * 2.0;
    let top = cell.y as f32 + (cell.h as f32 - side) / 2.0;
    grid.paint.push(Paint::Icon {
        bounds: Bounds::cells(middle - across / 2.0, top, across, side),
        icon,
        color,
    });
    let shown = Style {
        fg: color,
        painted: true,
        ..style
    };
    grid.put_str(cell.x + 1, cell.y, fallback(icon), shown, cell.right());
}

/// Draws the bar across `row`. `style` names the caret's paragraph, and `active` the formats
/// it's already in.
pub fn draw(
    grid: &mut Grid,
    row: Rect,
    (paragraph, active, tracking): (&str, &[Format], bool),
    bar: Bar,
    hover: Option<(u16, u16)>,
    theme: &Theme,
) {
    let base = theme.breadcrumb;
    let all = layout(row, bar);
    for button in &all {
        let cell = Rect::new(button.x, row.y, button.width, 1);
        let on_it = hover.is_some_and(|h| cell.contains(h.0, h.1));
        let on = match button.tool {
            Tool::Format(format) => active.contains(&format),
            Tool::Track => tracking,
            _ => false,
        };
        if on_it || on {
            let fill = if on { theme.pick } else { theme.popup_hover };
            grid.panel(Bounds::inset(cell, 0.12, 0.1), 16, Some(fill), None);
        }
        let color = if on || on_it {
            theme.text.fg
        } else {
            theme.status.fg
        };
        if button.tool == Tool::Styles {
            let shown = Style { fg: color, ..base };
            let right = cell.right().saturating_sub(2);
            grid.label_at(
                cell.x as f32 + 0.8,
                cell.x + 1,
                row.y,
                paragraph,
                shown,
                right,
                90,
            );
            let top = row.y as f32 + 0.25;
            let left = cell.right() as f32 - 1.8;
            grid.paint.push(Paint::Icon {
                bounds: Bounds::cells(left, top, 1.0, 0.5),
                icon: Icon::ChevronDown,
                color,
            });
            continue;
        }
        let face = buttons(bar)
            .chain([&EXPORT, &MORE, &BUILD])
            .find(|b| b.tool == button.tool)
            .map(|b| b.face);
        if let Some(face) = face {
            draw_face(grid, face, cell, color, base);
        }
    }
    for button in buttons(bar).filter(|b| b.gap) {
        if let Some(shown) = all.iter().find(|p| p.tool == button.tool) {
            let x = shown.x as f32 - GAP as f32 / 2.0;
            let line = Bounds::cells(x, row.y as f32 + 0.22, 0.0, 0.56);
            grid.hairline(line, theme.border);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notebooks_groups_are_divided_where_they_start() {
        let row = Rect::new(10, 1, 100, 1);
        let theme = Theme::named(Some("dark"));
        let mut grid = Grid::new(120, 3, theme.text);
        draw(
            &mut grid,
            row,
            ("", &[], false),
            Bar::Notebook,
            None,
            &theme,
        );
        let lines = grid
            .paint
            .iter()
            .filter(|p| matches!(p, Paint::Hairline { .. }));
        let gaps = NOTEBOOK_BUTTONS.iter().filter(|b| b.gap).count();
        assert_eq!(lines.count(), gaps);
    }

    #[test]
    fn a_wide_bar_shows_everything() {
        let row = Rect::new(10, 1, 100, 1);
        let buttons = layout(row, Bar::Page(Dialect::Markdown));
        assert_eq!(buttons.len(), BUTTONS.len() + 2);
        assert!(overflow(row, Bar::Page(Dialect::Markdown)).is_empty());
        assert_eq!(buttons.last().unwrap().tool, Tool::Export);
        assert_eq!(
            buttons.last().unwrap().x + WIDTH + 1,
            110,
            "Export ends the row"
        );
    }

    #[test]
    fn what_doesnt_fit_moves_to_the_menu() {
        let row = Rect::new(0, 1, 40, 1);
        let buttons = layout(row, Bar::Page(Dialect::Markdown));
        assert!(buttons.iter().any(|b| b.tool == Tool::More));
        let hidden = overflow(row, Bar::Page(Dialect::Markdown));
        assert_eq!(buttons.len() - 3 + hidden.len(), BUTTONS.len());
        assert!(hidden.iter().any(|(name, _)| *name == "Comment"));
        for pair in buttons.windows(2) {
            assert!(pair[0].x + pair[0].width <= pair[1].x, "{pair:?}");
        }
    }

    #[test]
    fn a_click_finds_its_button() {
        let row = Rect::new(0, 1, 100, 1);
        let bold = layout(row, Bar::Page(Dialect::Markdown))[1];
        let found = hit(row, bold.x + 1, Bar::Page(Dialect::Markdown));
        assert_eq!(found.map(|p| p.name), Some("Bold"));
        assert_eq!(hit(row, 70, Bar::Page(Dialect::Markdown)), None);
    }

    #[test]
    fn a_paper_cites_and_builds() {
        let row = Rect::new(0, 1, 100, 1);
        let tools: Vec<Tool> = layout(row, Bar::Page(Dialect::Latex))
            .iter()
            .map(|b| b.tool)
            .collect();
        assert!(tools.contains(&Tool::Cite));
        assert!(tools.contains(&Tool::Build));
        assert!(!tools.contains(&Tool::Format(Format::Task)));
    }
}
