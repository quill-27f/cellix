//! Markdown as one editable page, the way Xcode's rendered editor works. The source stays
//! the document: this lays it out with its markers hidden, and every visible character
//! remembers its source position, so clicks and edits land in the right place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use cellix_doc::latex::Project;
use cellix_lang::{Registry, Token};
use cellix_render::{Bitmap, Font, INTERFACE_SIZE, Measure, Pixels};

mod tex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Text,
    Heading,
    /// Inline code, on a pill.
    Code,
    /// Text in a code block, coloured by the language if it's known.
    Block(Option<Token>),
    Link,
    Dim,
    /// A task checkbox, ticked or not. The pixel tiers draw it as a shape.
    Check(bool),
    /// Space or a table's column line, which the pixel tiers don't paint as text.
    TableLine,
    /// A formula's source while it's being edited.
    Source,
    /// A typeset formula, as an index into the page's figures, and its width in 64ths.
    Formula {
        index: u16,
        width: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    tone: Tone,
    font: Font,
    underline: bool,
}

const PLAIN: Look = Look {
    tone: Tone::Text,
    font: Font {
        mono: false,
        bold: false,
        italic: false,
        size: 100,
    },
    underline: false,
};

/// One visible character. `at` is its source position, and None marks decoration such as
/// a bullet, which has no source of its own. `x` and `advance` are in cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub at: Option<usize>,
    pub x: f32,
    pub advance: f32,
    pub tone: Tone,
    pub font: Font,
    pub underline: bool,
}

fn glyph(ch: char, at: Option<usize>, look: Look) -> Glyph {
    Glyph {
        ch,
        at,
        x: 0.0,
        advance: 0.0,
        tone: look.tone,
        font: look.font,
        underline: look.underline,
    }
}

fn decoration(text: &str, tone: Tone) -> Vec<Glyph> {
    let look = Look { tone, ..PLAIN };
    text.chars().map(|c| glyph(c, None, look)).collect()
}

/// Fills in each glyph's advance, measuring runs of one font together so kerning counts.
fn measure_glyphs(glyphs: &mut [Glyph], measure: &dyn Measure) {
    let mut start = 0;
    while start < glyphs.len() {
        let font = glyphs[start].font;
        let end = (start..glyphs.len())
            .find(|i| glyphs[*i].font != font)
            .unwrap_or(glyphs.len());
        let text: String = glyphs[start..end].iter().map(|g| g.ch).collect();
        let advances = measure.advances(&text, font);
        for (g, advance) in glyphs[start..end].iter_mut().zip(advances.iter()) {
            g.advance = match g.tone {
                Tone::Formula { width, .. } => width as f32 / 64.0,
                _ => *advance,
            };
        }
        start = end;
    }
}

/// A table row's geometry, for drawing its box.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    /// The left edge of each column, then the right edge of the table.
    pub columns: Vec<f32>,
    /// Where an empty cell's caret stands in each column, which alignment moves.
    pub carets: Vec<f32>,
    pub header: bool,
}

/// One screen row of the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub glyphs: Vec<Glyph>,
    /// Source line, counting from zero.
    pub line: usize,
    /// Where the caret goes at this row's end.
    pub end: usize,
    /// Where text starts, for a caret on an empty row.
    pub lead: f32,
    /// Whether the caret may stop here. Labels, spacers and the like aren't text.
    pub stop: bool,
    /// The last row of its source line, when a line wraps.
    pub last: bool,
    /// Part of a code block.
    pub shaded: bool,
    /// A horizontal rule.
    pub rule: bool,
    /// Has a quote's bar.
    pub quote: bool,
    /// A heading's level, or zero.
    pub heading: u8,
    /// Text stands two rows tall, centred across this row and an empty one under it.
    pub tall: bool,
    /// The column of a task checkbox, and the source position of its mark.
    pub checkbox: Option<(f32, usize)>,
    /// Where each table cell starts in the source.
    pub cells: Vec<usize>,
    pub table: Option<TableRow>,
    /// A picture that starts on this row, as an index into the page's figures, and its
    /// column.
    pub figure: Option<(usize, f32)>,
    /// What clicking this row opens, as a path or a place in the document.
    pub target: Option<Target>,
}

/// What a clickable row leads to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    File(PathBuf),
    Preamble,
}

impl Row {
    fn new(line: usize, end: usize) -> Self {
        Self {
            glyphs: Vec::new(),
            line,
            end,
            lead: 0.0,
            stop: true,
            last: true,
            shaded: false,
            rule: false,
            quote: false,
            heading: 0,
            tall: false,
            checkbox: None,
            cells: Vec::new(),
            table: None,
            figure: None,
            target: None,
        }
    }

    /// The cell a source position is in.
    fn cell_holding(&self, pos: usize) -> Option<usize> {
        self.cells.iter().rposition(|start| *start <= pos)
    }

    /// Whether a cell shows nothing, which leaves its start the only way to reach it.
    fn cell_is_empty(&self, index: usize) -> bool {
        let start = self.cells[index];
        let end = self.cells.get(index + 1).copied().unwrap_or(usize::MAX);
        let mut places = self.glyphs.iter().filter_map(|g| g.at);
        !places.any(|at| (start..end).contains(&at))
    }

    fn caret_x(&self, pos: usize) -> f32 {
        if let (Some(table), Some(index)) = (&self.table, self.cell_holding(pos))
            && self.cell_is_empty(index)
            && let Some(x) = table.carets.get(index)
        {
            return *x;
        }
        let mut before: Option<(usize, f32)> = None;
        for g in &self.glyphs {
            if g.at.is_some_and(|at| at >= pos) {
                return g.x;
            }
            // The end of a table cell is a place of its own, before the next column.
            if let (None, Some((end, x))) = (g.at, before)
                && end == pos
            {
                return x;
            }
            before = g.at.map(|at| (at + 1, g.x + g.advance));
        }
        before.map_or(self.lead, |(_, x)| x)
    }

    /// The source position nearest a column.
    pub fn position_at(&self, column: f32) -> usize {
        // A picture has one place: inside its source.
        if let (Some(_), Some(at)) = (self.figure, self.glyphs.first().and_then(|g| g.at)) {
            return at;
        }
        // Past the table's edge still means its last column.
        let cell = self.table.as_ref().and_then(|t| {
            let last = t.columns.len().checked_sub(2)?;
            let index = t.columns.iter().rposition(|x| *x <= column)?.min(last);
            Some((t.columns[index], t.columns[index + 1], index))
        });
        let in_scope = |g: &&Glyph| {
            g.at.is_some() && cell.is_none_or(|(left, right, _)| g.x >= left && g.x < right)
        };
        let mut last = None;
        for g in self.glyphs.iter().filter(in_scope) {
            if column < g.x + g.advance / 2.0 {
                return g.at.unwrap_or(self.end);
            }
            last = g.at;
        }
        match (last, cell) {
            (Some(at), Some(_)) => at + 1,
            (None, Some((.., index))) => self.cells.get(index).copied().unwrap_or(self.end),
            _ => self.end,
        }
    }
}

/// How a line starts, with its hidden marker.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Marker {
    Heading(usize),
    Quote,
    Bullet,
    Ordered(u64),
    Task { checked: bool, mark: usize },
}

struct Prefix {
    marker: Marker,
    indent: usize,
    /// Chars of source the marker takes up, including indent.
    hidden: usize,
}

fn prefix(line: &str) -> Option<Prefix> {
    let chars: Vec<char> = line.chars().collect();
    let indent = chars.iter().take_while(|c| **c == ' ').count();
    let rest = &chars[indent..];
    let hashes = rest.iter().take_while(|c| **c == '#').count();
    if (1..=6).contains(&hashes) && rest.get(hashes) == Some(&' ') {
        return Some(Prefix {
            marker: Marker::Heading(hashes),
            indent,
            hidden: indent + hashes + 1,
        });
    }
    if rest.len() >= 2 && rest[0] == '>' && rest[1] == ' ' {
        return Some(Prefix {
            marker: Marker::Quote,
            indent,
            hidden: indent + 2,
        });
    }
    if rest.len() >= 2 && matches!(rest[0], '-' | '*' | '+') && rest[1] == ' ' {
        if rest.len() >= 6
            && rest[2] == '['
            && matches!(rest[3], ' ' | 'x' | 'X')
            && rest[4] == ']'
            && rest[5] == ' '
        {
            return Some(Prefix {
                marker: Marker::Task {
                    checked: rest[3] != ' ',
                    mark: indent + 3,
                },
                indent,
                hidden: indent + 6,
            });
        }
        return Some(Prefix {
            marker: Marker::Bullet,
            indent,
            hidden: indent + 2,
        });
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && rest.get(digits) == Some(&'.') && rest.get(digits + 1) == Some(&' ') {
        let number: String = rest[..digits].iter().collect();
        return Some(Prefix {
            marker: Marker::Ordered(number.parse().unwrap_or(1)),
            indent,
            hidden: indent + digits + 2,
        });
    }
    None
}

/// Whether the line after `index` carries on the same paragraph.
fn continues_paragraph(lines: &[(usize, &str)], index: usize) -> bool {
    let (Some((_, line)), Some((_, next))) = (lines.get(index), lines.get(index + 1)) else {
        return false;
    };
    // Two trailing spaces or a backslash are Markdown's way of forcing a line break.
    let hard_break = line.ends_with("  ") || line.ends_with('\\');
    let starts_table =
        next.contains('|') && lines.get(index + 2).is_some_and(|n| is_table_rule(n.1));
    !hard_break
        && !next.trim().is_empty()
        && !next.trim_start().starts_with("```")
        && !next.trim_start().starts_with("$$")
        && !next.trim_start().starts_with("\\[")
        && image_line(next).is_none()
        && !is_rule(next)
        && !starts_table
        && prefix(next).is_none()
}

fn is_rule(line: &str) -> bool {
    let text = line.trim();
    text.len() >= 3
        && (text.chars().all(|c| c == '-')
            || text.chars().all(|c| c == '*')
            || text.chars().all(|c| c == '_'))
}

/// A typeset formula or an image, sized for the page.
#[derive(Clone, Debug, PartialEq)]
pub struct Figure {
    pub bitmap: Bitmap,
    /// Its size in cells.
    pub width: f32,
    pub height: f32,
    /// How far its baseline sits below its top, in rows.
    pub baseline: f32,
}

/// Pictures already made, kept between layouts: a page is laid out on every key.
#[derive(Default)]
pub struct Library {
    formulas: HashMap<(String, bool), Option<Figure>>,
    /// The last formula that typeset at each place, to hold while its source is half typed.
    last_good: HashMap<usize, Figure>,
    images: HashMap<(PathBuf, u32), Option<Figure>>,
}

/// What a page is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dialect {
    #[default]
    Markdown,
    Latex,
}

impl Dialect {
    /// The dialect of a file, if it opens as a page.
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "md" | "markdown" | "mdx" => Some(Self::Markdown),
            "tex" | "ltx" => Some(Self::Latex),
            _ => None,
        }
    }
}

/// What a page needs beyond its text.
#[derive(Default)]
pub struct Extras<'a> {
    pub dialect: Dialect,
    /// The paper this file belongs to.
    pub project: Option<Rc<Project>>,
    /// The file being laid out.
    pub file: Option<&'a Path>,
    /// Shows a paper's preamble as source.
    pub preamble: bool,
    /// The span showing as source, because the caret is in it.
    pub open: Option<(usize, usize)>,
    /// Where relative image paths start.
    pub folder: Option<&'a Path>,
    pub library: Option<&'a RefCell<Library>>,
    /// Filled in by layout.
    pub figures: RefCell<Vec<Figure>>,
    /// The picture of a formula being edited in a line, filled in by layout.
    pub preview: RefCell<Option<usize>>,
    /// Every formula and image in the page, as source ranges, filled in by layout.
    pub spans: RefCell<Vec<(usize, usize)>>,
}

/// Formulas are set at the size of the text around them, so they read as part of it.
fn figure_of(formula: &cellix_doc::math::Formula, cell: (u32, u32)) -> Figure {
    Figure {
        bitmap: Bitmap {
            id: formula.id,
            width: formula.width,
            height: formula.height,
            kind: Pixels::Coverage,
            pixels: formula.alpha.clone(),
        },
        width: formula.width as f32 / cell.0 as f32,
        height: formula.height as f32 / cell.1 as f32,
        baseline: formula.baseline as f32 / cell.1 as f32,
    }
}

/// Reads an image and sets it to fit the page, keeping its shape.
fn load_image(path: &Path, cell: (u32, u32), width: f32) -> Option<Figure> {
    let wanted = (width * cell.0 as f32) as u32;
    let image = if path.extension().is_some_and(|e| e == "pdf") {
        let document = cellix_doc::pdf::Document::open(std::fs::read(path).ok()?).ok()?;
        let page = document.render(0, wanted)?;
        let pixels = page.rgba.as_ref().clone();
        image::RgbaImage::from_raw(page.width, page.height, pixels)?
    } else {
        image::open(path).ok()?.to_rgba8()
    };
    let image = if image.width() > wanted {
        let height = image.height() * wanted / image.width();
        image::imageops::thumbnail(&image, wanted, height.max(1))
    } else {
        image
    };
    let (w, h) = image.dimensions();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&path, &mut hasher);
    std::hash::Hash::hash(&(w, h), &mut hasher);
    Some(Figure {
        bitmap: Bitmap {
            id: std::hash::Hasher::finish(&hasher),
            width: w,
            height: h,
            kind: Pixels::Rgba,
            pixels: Arc::new(image.into_raw()),
        },
        width: w as f32 / cell.0 as f32,
        height: h as f32 / cell.1 as f32,
        baseline: 0.0,
    })
}

/// Finds where inline math closes: the closing `$` hugs its text and isn't a price.
fn math_end(chars: &[char]) -> Option<usize> {
    if chars.get(1).is_none_or(|c| c.is_whitespace() || *c == '$') {
        return None;
    }
    (2..chars.len()).find(|i| {
        chars[*i] == '$'
            && chars[*i - 1] != '\\'
            && !chars[*i - 1].is_whitespace()
            && !chars.get(*i + 1).is_some_and(char::is_ascii_digit)
    })
}

fn extras_preview(context: &Context, figure: Option<usize>) {
    *context.extras.preview.borrow_mut() = figure;
}

/// Finds a closing marker in `chars` at or after `from`.
fn closing(chars: &[char], from: usize, marker: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(marker.len() - 1))
        .find(|i| chars[*i..].starts_with(marker) && *i > from)
}

/// Turns one span of text into glyphs, hiding emphasis, code and link markers.
fn inline(chars: &[char], offset: usize, look: Look, context: &Context, out: &mut Vec<Glyph>) {
    let mut i = 0;
    while i < chars.len() {
        let rest = &chars[i..];
        let at = offset + i;
        let wrap = |marker: &[char], next: Look, out: &mut Vec<Glyph>| {
            let open = marker.len();
            let close = closing(rest, open, marker)?;
            inline(&rest[open..close], at + open, next, context, out);
            Some(close + open)
        };
        let bold = Look {
            font: Font {
                bold: true,
                ..look.font
            },
            ..look
        };
        let italic = Look {
            font: Font {
                italic: true,
                ..look.font
            },
            ..look
        };
        let used = if rest.starts_with(&['*', '*']) {
            wrap(&['*', '*'], bold, out)
        } else if rest.starts_with(&['_', '_']) {
            wrap(&['_', '_'], bold, out)
        } else if rest[0] == '*' && rest.get(1).is_some_and(|c| *c != ' ') {
            wrap(&['*'], italic, out)
        } else if rest[0] == '_'
            && rest.get(1).is_some_and(|c| *c != ' ')
            && (i == 0 || !chars[i - 1].is_alphanumeric())
        {
            wrap(&['_'], italic, out)
        } else if rest[0] == '`' {
            closing(rest, 1, &['`']).map(|close| {
                let code = Look {
                    tone: Tone::Code,
                    font: Font {
                        mono: true,
                        ..look.font
                    },
                    ..look
                };
                for (n, c) in rest[1..close].iter().enumerate() {
                    out.push(glyph(*c, Some(at + 1 + n), code));
                }
                close + 1
            })
        } else if rest[0] == '[' {
            link(rest, at, look, context, out)
        } else if rest[0] == '$' && (i == 0 || chars[i - 1] != '\\') {
            math_end(rest).map(|end| {
                formula_span(&rest[..=end], 1, at, context, out);
                end + 1
            })
        } else {
            None
        };
        match used {
            Some(count) => i += count,
            None => {
                out.push(glyph(chars[i], Some(at), look));
                i += 1;
            }
        }
    }
}

/// `[text](url)`: shows the text and hides the rest.
fn link(
    rest: &[char],
    at: usize,
    look: Look,
    context: &Context,
    out: &mut Vec<Glyph>,
) -> Option<usize> {
    let close = rest.iter().position(|c| *c == ']')?;
    if rest.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = close + 1 + rest[close + 1..].iter().position(|c| *c == ')')?;
    let shown = Look {
        tone: Tone::Link,
        underline: true,
        ..look
    };
    inline(&rest[1..close], at + 1, shown, context, out);
    Some(end + 1)
}

/// A formula in running text, with delimiters `edge` chars long: a picture of it, or its
/// source when the caret is in it, when it won't typeset, or on the cell tier.
fn formula_span(span: &[char], edge: usize, at: usize, context: &Context, out: &mut Vec<Glyph>) {
    let range = (at, at + span.len());
    context.extras.spans.borrow_mut().push(range);
    let body = &span[edge..span.len() - edge];
    let inner: String = body.iter().collect();
    let code = Look {
        tone: Tone::Source,
        font: Font {
            mono: true,
            ..Font::default()
        },
        underline: false,
    };
    if context.extras.open == Some(range) {
        for (n, c) in span.iter().enumerate() {
            out.push(glyph(*c, Some(at + n), code));
        }
        // What it'll look like, while it's being typed.
        let shown = context.formula_or_last(&inner, false, at);
        extras_preview(context, shown);
        return;
    }
    let Some(figure) = context.formula(&inner, false) else {
        for (n, c) in body.iter().enumerate() {
            out.push(glyph(*c, Some(at + edge + n), code));
        }
        return;
    };
    let width = (context.extras.figures.borrow()[figure].width * 64.0).ceil() as u16;
    let tone = Tone::Formula {
        index: figure as u16,
        width,
    };
    // Inside the span, so reaching it from either side opens it for editing.
    out.push(glyph('\u{FFFC}', Some(at + 1), Look { tone, ..PLAIN }));
}

/// Sets a line's glyphs into rows no wider than `width`, breaking at spaces, with
/// continuation rows starting at `lead`.
fn wrap(template: Row, glyphs: Vec<Glyph>, lead: f32, width: f32, rows: &mut Vec<Row>) {
    let end = template.end;
    let mut pieces: Vec<Vec<Glyph>> = Vec::new();
    let mut current: Vec<Glyph> = Vec::new();
    let mut x = 0.0;
    for mut g in glyphs {
        if x + g.advance > width && !current.is_empty() {
            let cut = current
                .iter()
                .rposition(|g| g.ch == ' ' && g.at.is_some())
                .filter(|i| current[*i].x > lead)
                .map_or(current.len(), |i| i + 1);
            let mut tail = current.split_off(cut);
            pieces.push(std::mem::take(&mut current));
            x = lead;
            for t in &mut tail {
                t.x = x;
                x += t.advance;
            }
            current = tail;
        }
        g.x = x;
        x += g.advance;
        current.push(g);
    }
    pieces.push(current);
    let count = pieces.len();
    let starts: Vec<usize> = pieces
        .iter()
        .map(|p| p.iter().find_map(|g| g.at).unwrap_or(end))
        .collect();
    for (i, glyphs) in pieces.into_iter().enumerate() {
        let last = i + 1 == count;
        rows.push(Row {
            glyphs,
            end: starts.get(i + 1).copied().unwrap_or(end),
            last,
            lead,
            ..template.clone()
        });
    }
}

fn split_cells(line: &str) -> Vec<(usize, Vec<char>)> {
    let chars: Vec<char> = line.chars().collect();
    let mut cells = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i <= chars.len() {
        if i == chars.len() || chars[i] == '|' {
            if i > 0 || (i == 0 && chars.first() != Some(&'|')) {
                cells.push((start, chars[start..i].to_vec()));
            }
            start = i + 1;
        }
        i += 1;
    }
    if line.trim_end().ends_with('|') {
        cells.pop();
    }
    cells
        .into_iter()
        .map(|(start, cell)| {
            let lead = cell.iter().take_while(|c| **c == ' ').count();
            // An empty cell's caret stands one space in, so typed text stays clear of the bar.
            if lead == cell.len() {
                return (start + lead.min(1), Vec::new());
            }
            let tail = cell.iter().rev().take_while(|c| **c == ' ').count();
            let end = cell.len().saturating_sub(tail).max(lead);
            (start + lead, cell[lead..end].to_vec())
        })
        .collect()
}

fn is_table_rule(line: &str) -> bool {
    let text = line.trim();
    text.contains('-') && text.contains('|') && text.chars().all(|c| "|-: ".contains(c))
}

/// Lays out a table with its pipes and rule hidden: real columns, with padding.
fn table(lines: &[(usize, &str)], first: usize, context: &Context, rows: &mut Vec<Row>) {
    let mut parsed: Vec<Vec<Vec<Glyph>>> = Vec::new();
    let mut starts: Vec<Vec<usize>> = Vec::new();
    for (index, (offset, line)) in lines.iter().enumerate() {
        if index == 1 {
            continue;
        }
        let look = Look {
            font: Font {
                bold: index == 0,
                ..Font::default()
            },
            ..PLAIN
        };
        let cells = split_cells(line);
        starts.push(cells.iter().map(|(at, _)| offset + at).collect());
        parsed.push(
            cells
                .iter()
                .map(|(at, cell)| {
                    let mut out = Vec::new();
                    inline(cell, offset + at, look, context, &mut out);
                    measure_glyphs(&mut out, context.measure);
                    out
                })
                .collect(),
        );
    }
    let places: Vec<(usize, usize)> = (0..parsed.len())
        .map(|index| {
            let source = if index == 0 { 0 } else { index + 1 };
            (
                first + source,
                lines[source].0 + lines[source].1.chars().count(),
            )
        })
        .collect();
    let aligns: Vec<char> = split_cells(lines[1].1)
        .iter()
        .map(|(_, cell)| match (cell.first(), cell.last()) {
            (Some(':'), Some(':')) => 'c',
            (_, Some(':')) => 'r',
            _ => 'l',
        })
        .collect();
    place_table(parsed, starts, &places, &aligns, rows);
}

/// Sets a table's cells in columns. Each row has its line and where that line ends.
fn place_table(
    parsed: Vec<Vec<Vec<Glyph>>>,
    starts: Vec<Vec<usize>>,
    places: &[(usize, usize)],
    aligns: &[char],
    rows: &mut Vec<Row>,
) {
    /// Space between a cell's edge and its text, in cells.
    const PADDING: f32 = 1.0;
    let count = parsed.iter().map(Vec::len).max().unwrap_or(0);
    let mut columns = vec![0.0f32];
    for c in 0..count {
        let widest = parsed
            .iter()
            .filter_map(|r| r.get(c))
            .map(|cell| cell.iter().map(|g| g.advance).sum::<f32>())
            .fold(3.0f32, f32::max);
        columns.push(columns[c] + widest.ceil() + PADDING * 2.0);
    }
    let shift = |c: usize, room: f32| match aligns.get(c) {
        Some('r') => room,
        Some('c') => room / 2.0,
        _ => 0.0,
    };
    let carets: Vec<f32> = (0..count)
        .map(|c| columns[c] + PADDING + shift(c, columns[c + 1] - columns[c] - PADDING * 2.0))
        .collect();
    for (index, cells) in parsed.iter().enumerate() {
        let (line, line_end) = places[index];
        let mut glyphs = Vec::new();
        for (c, left) in columns.iter().take(count).enumerate() {
            if c > 0 {
                let mut bar = decoration("│", Tone::TableLine);
                bar[0].x = *left;
                bar[0].advance = 1.0;
                glyphs.extend(bar);
            }
            let width: f32 = cells.get(c).into_iter().flatten().map(|g| g.advance).sum();
            let room = (columns[c + 1] - left - PADDING * 2.0 - width).max(0.0);
            let mut x = left + PADDING + shift(c, room);
            for g in cells.get(c).into_iter().flatten() {
                glyphs.push(Glyph { x, ..*g });
                x += g.advance;
            }
        }
        // The row runs to its last cell, written in or not, and never past the closing bar.
        let written = cells.iter().flatten().filter_map(|g| g.at).max();
        let end = written
            .map(|at| at + 1)
            .into_iter()
            .chain(starts[index].last().copied())
            .max()
            .unwrap_or(line_end);
        rows.push(Row {
            glyphs,
            cells: starts[index].clone(),
            lead: PADDING,
            table: Some(TableRow {
                columns: columns.clone(),
                carets: carets.clone(),
                header: index == 0,
            }),
            ..Row::new(line, end)
        });
    }
}

/// The file extension the registry knows a fence's language by.
fn fence_extension(language: &str) -> &str {
    match language {
        "rust" => "rs",
        "python" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "shell" | "bash" | "zsh" => "sh",
        other => other,
    }
}

/// Space inside a code block, in cells.
const CODE_PADDING: f32 = 1.2;

struct Context<'a> {
    measure: &'a dyn Measure,
    registry: &'a Registry,
    extras: &'a Extras<'a>,
}

impl Context<'_> {
    /// A cell's pixels, or None on the cell tier, which has no pictures.
    fn cell(&self) -> Option<(u32, u32)> {
        self.measure.cell_pixels()
    }

    fn keep(&self, figure: Figure) -> usize {
        let mut figures = self.extras.figures.borrow_mut();
        figures.push(figure);
        figures.len() - 1
    }

    /// Typesets a formula for this page, and returns its index among the figures.
    fn formula(&self, source: &str, display: bool) -> Option<usize> {
        let cell = self.cell()?;
        let expanded = self.extras.project.as_ref().map(|p| p.expand(source));
        let source = expanded.as_deref().unwrap_or(source);
        let make = || {
            // Display math stands a little larger, as it does in print.
            let scale = if display { 1.2 } else { 1.05 };
            let em = (cell.1 as f32 * INTERFACE_SIZE * scale).round() as u32;
            let formula = cellix_doc::math::render(source, display, em).ok()?;
            Some(figure_of(&formula, cell))
        };
        let figure = match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .formulas
                .entry((source.to_string(), display))
                .or_insert_with(make)
                .clone(),
            None => make(),
        };
        figure.map(|f| self.keep(f))
    }

    /// A formula, or the last good one at this place. Typing `\\frac{` shouldn't make the
    /// picture vanish and the page jump until the brace closes.
    fn formula_or_last(&self, source: &str, display: bool, place: usize) -> Option<usize> {
        let found = self.formula(source, display);
        let Some(library) = self.extras.library else {
            return found;
        };
        match found {
            Some(index) => {
                let figure = self.extras.figures.borrow()[index].clone();
                library.borrow_mut().last_good.insert(place, figure);
                Some(index)
            }
            None => {
                let figure = library.borrow().last_good.get(&place).cloned()?;
                Some(self.keep(figure))
            }
        }
    }

    /// Why a formula won't typeset.
    fn formula_error(&self, source: &str) -> Option<String> {
        let expanded = self.extras.project.as_ref().map(|p| p.expand(source));
        let source = expanded.as_deref().unwrap_or(source);
        cellix_doc::math::render(source, true, 20).err()
    }

    /// Loads an image for this page, and returns its index among the figures. LaTeX names
    /// files without their extension, and a figure is often a PDF.
    fn image(&self, source: &str, width: f32) -> Option<usize> {
        let cell = self.cell()?;
        let folder = self.extras.folder?;
        let named = folder.join(source);
        let path = ["", "png", "jpg", "jpeg", "pdf", "webp"]
            .iter()
            .map(|extension| {
                if extension.is_empty() {
                    named.clone()
                } else {
                    named.with_extension(extension)
                }
            })
            .find(|path| path.is_file())?;
        let make = || load_image(&path, cell, width);
        let figure = match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .images
                .entry((path.clone(), width as u32))
                .or_insert_with(make)
                .clone(),
            None => make(),
        };
        figure.map(|f| self.keep(f))
    }
}

/// `$$` and `\\[` blocks: the line a block ends on, and the formula inside it.
fn math_block(lines: &[(usize, &str)], first: usize) -> Option<(usize, String)> {
    let line = lines[first].1.trim();
    let (open, close) = if line.starts_with("$$") {
        ("$$", "$$")
    } else if line.starts_with("\\[") {
        ("\\[", "\\]")
    } else {
        return None;
    };
    let after = &line[open.len()..];
    if let Some(inner) = after.strip_suffix(close) {
        return Some((first, inner.trim().to_string()));
    }
    let mut inner = vec![after];
    for (index, (_, text)) in lines.iter().enumerate().skip(first + 1) {
        let text = text.trim();
        if let Some(last) = text.strip_suffix(close) {
            inner.push(last);
            return Some((index, inner.join("\n").trim().to_string()));
        }
        inner.push(text);
    }
    None
}

/// `![alt](path)` standing alone on its line.
fn image_line(line: &str) -> Option<(usize, &str, &str)> {
    let text = line.trim();
    let rest = text.strip_prefix("![")?.strip_suffix(')')?;
    let (alt, path) = rest.split_once("](")?;
    (!path.contains(' ')).then_some((line.len() - line.trim_start().len(), alt, path))
}

/// Sets equation numbers at the right margin, spread down the picture's rows.
fn margin_numbers(
    numbers: &[Option<String>],
    first: usize,
    height: f32,
    width: f32,
    context: &Context,
    rows: &mut [Row],
) {
    let count = numbers.len().max(1) as f32;
    for (index, number) in numbers.iter().enumerate() {
        let Some(number) = number else {
            continue;
        };
        // In the middle of its share of the picture, which starts a little into its row.
        let middle = 0.2 + height * (index as f32 + 0.5) / count;
        let row = first + middle.floor() as usize;
        let number = number.replace('$', "").replace("\\ast", "∗");
        let mut mark = decoration(&format!("({number})"), Tone::Dim);
        measure_glyphs(&mut mark, context.measure);
        let shown: f32 = mark.iter().map(|g| g.advance).sum();
        let mut x = width - shown;
        for g in &mut mark {
            g.x = x;
            x += g.advance;
        }
        if let Some(row) = rows.get_mut(row) {
            row.glyphs.extend(mark);
        }
    }
}

/// A picture's rows: the first carries it and a caret stop, and the rest make its room.
fn figure_rows(
    figure: usize,
    context: &Context,
    (line, at, end): (usize, usize, usize),
    width: f32,
    rows: &mut Vec<Row>,
) {
    let (shape_width, height) = {
        let figures = context.extras.figures.borrow();
        (figures[figure].width, figures[figure].height)
    };
    let mut stop = glyph(
        ' ',
        Some(at),
        Look {
            tone: Tone::TableLine,
            ..PLAIN
        },
    );
    stop.advance = 0.0;
    rows.push(Row {
        glyphs: vec![stop],
        figure: Some((figure, ((width - shape_width) / 2.0).max(0.0))),
        ..Row::new(line, end)
    });
    // A click anywhere on the picture means the picture.
    for _ in 1..(height + 0.4).ceil() as usize {
        rows.push(spacer(line, at));
    }
}

/// Rows for a block of code, coloured by its language.
fn code_rows(
    lines: &[(usize, &str)],
    first: usize,
    language: &str,
    context: &Context,
    tone: Option<Tone>,
    rows: &mut Vec<Row>,
) {
    let source: String = lines.iter().map(|(_, l)| format!("{l}\n")).collect();
    let path = format!("code.{}", fence_extension(language));
    let spans = context.registry.highlight(Path::new(&path), &source);
    let mut byte = 0;
    for (index, (offset, line)) in lines.iter().enumerate() {
        let mut glyphs = Vec::new();
        for (i, ch) in line.chars().enumerate() {
            let token = spans
                .iter()
                .find(|s| s.start <= byte && byte < s.end)
                .map(|s| s.token);
            let look = Look {
                tone: tone.unwrap_or(Tone::Block(token)),
                font: Font {
                    mono: true,
                    size: 92,
                    ..Font::default()
                },
                underline: false,
            };
            glyphs.push(glyph(ch, Some(offset + i), look));
            byte += ch.len_utf8();
        }
        byte += 1;
        measure_glyphs(&mut glyphs, context.measure);
        let mut x = CODE_PADDING;
        for g in &mut glyphs {
            g.x = x;
            x += g.advance;
        }
        rows.push(Row {
            glyphs,
            shaded: true,
            lead: CODE_PADDING,
            ..Row::new(first + index, offset + line.chars().count())
        });
    }
}

/// Sizes of headings one to six, in percent of body text.
const HEADINGS: [u8; 6] = [190, 150, 122, 110, 100, 100];

fn spacer(line: usize, end: usize) -> Row {
    Row {
        stop: false,
        ..Row::new(line, end)
    }
}

/// Lays the whole document out for a page `width` cells wide.
#[cfg(test)]
pub fn layout(text: &str, width: usize, measure: &dyn Measure, registry: &Registry) -> Vec<Row> {
    layout_with(text, width, measure, registry, &Extras::default())
}

/// Lays a document out, with its formulas and images.
pub fn layout_with(
    text: &str,
    width: usize,
    measure: &dyn Measure,
    registry: &Registry,
    extras: &Extras,
) -> Vec<Row> {
    let context = Context {
        measure,
        registry,
        extras,
    };
    let proportional = !measure.in_cells();
    let width = width as f32;
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        lines.push((offset, line));
        offset += line.chars().count() + 1;
    }
    let mut rows = Vec::new();
    if proportional {
        rows.push(spacer(0, 0));
    }
    if extras.dialect == Dialect::Latex {
        tex::layout(&lines, width, &context, &mut rows);
        return rows;
    }
    let mut i = 0;
    // Front matter shows as dimmed text, with its delimiters hidden.
    if lines[0].1 == "---"
        && let Some(close) = (1..lines.len()).find(|n| lines[*n].1 == "---")
    {
        code_rows(
            &lines[1..close],
            1,
            "",
            &context,
            Some(Tone::Dim),
            &mut rows,
        );
        i = close + 1;
    }
    while i < lines.len() {
        let (offset, line) = lines[i];
        let trimmed = line.trim_start();
        if let Some(language) = trimmed.strip_prefix("```") {
            let language = language.trim();
            let close = (i + 1..lines.len())
                .find(|n| lines[*n].1.trim_start().starts_with("```"))
                .unwrap_or(lines.len());
            if !language.is_empty() {
                let mut label = decoration(language, Tone::Dim);
                for g in &mut label {
                    g.font.size = 78;
                }
                measure_glyphs(&mut label, measure);
                let mut x = CODE_PADDING;
                for g in &mut label {
                    g.x = x;
                    x += g.advance;
                }
                rows.push(Row {
                    glyphs: label,
                    shaded: true,
                    ..spacer(i, offset)
                });
            }
            code_rows(
                &lines[i + 1..close],
                i + 1,
                language,
                &context,
                None,
                &mut rows,
            );
            i = close + 1;
            continue;
        }
        if line.contains('|') && lines.get(i + 1).is_some_and(|n| is_table_rule(n.1)) {
            let end = (i + 2..lines.len())
                .find(|n| !lines[*n].1.contains('|'))
                .unwrap_or(lines.len());
            table(&lines[i..end], i, &context, &mut rows);
            i = end;
            continue;
        }
        if let Some((last, formula)) = math_block(&lines, i) {
            let end = lines[last].0 + lines[last].1.chars().count();
            extras.spans.borrow_mut().push((offset, end + 1));
            let open = extras.open == Some((offset, end + 1));
            if open || context.cell().is_none() {
                code_rows(&lines[i..=last], i, "", &context, None, &mut rows);
            }
            if let Some(figure) = context.formula_or_last(&formula, true, offset) {
                // Opening still shows the result, under the source being typed.
                let inside = if last > i { lines[i + 1].0 } else { offset + 2 };
                let at = rows.len();
                figure_rows(figure, &context, (i, inside, end), width, &mut rows);
                let mut count = 0;
                let numbers = cellix_doc::latex::equation_rows(&formula, false, &mut count);
                let shown = context.extras.figures.borrow()[figure].height;
                margin_numbers(&numbers, at, shown, width, &context, &mut rows);
                if open {
                    let start = rows.iter().rposition(|r| r.figure.is_some());
                    if let Some(row) = start.and_then(|at| rows.get_mut(at)) {
                        row.stop = false;
                    }
                }
            } else if !open && context.cell().is_some() {
                // It won't typeset: the source is what there is.
                code_rows(
                    &lines[i..=last],
                    i,
                    "",
                    &context,
                    Some(Tone::Dim),
                    &mut rows,
                );
            }
            i = last + 1;
            continue;
        }
        if let Some((indent, alt, path)) = image_line(line) {
            let end = offset + line.chars().count();
            extras.spans.borrow_mut().push((offset, end + 1));
            let open = extras.open == Some((offset, end + 1));
            let figure = context.image(path, width.min(90.0));
            if let Some(figure) = figure.filter(|_| !open) {
                figure_rows(
                    figure,
                    &context,
                    (i, offset + indent + 1, end),
                    width,
                    &mut rows,
                );
                if !alt.is_empty() {
                    let mut caption = decoration(alt, Tone::Dim);
                    for g in &mut caption {
                        g.font.size = 85;
                    }
                    measure_glyphs(&mut caption, measure);
                    let shown: f32 = caption.iter().map(|g| g.advance).sum();
                    let mut x = ((width - shown) / 2.0).max(0.0);
                    for g in &mut caption {
                        g.x = x;
                        x += g.advance;
                    }
                    rows.push(Row {
                        glyphs: caption,
                        ..spacer(i, end)
                    });
                }
                i += 1;
                continue;
            }
            if open {
                code_rows(&lines[i..=i], i, "", &context, None, &mut rows);
                i += 1;
                continue;
            }
        }
        if is_rule(line) {
            let mut glyphs = decoration(&"─".repeat(width as usize), Tone::Dim);
            measure_glyphs(&mut glyphs, measure);
            let mut x = 0.0;
            for g in &mut glyphs {
                g.x = x;
                x += g.advance;
            }
            rows.push(Row {
                glyphs,
                rule: true,
                ..Row::new(i, offset)
            });
            i += 1;
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let found = prefix(line);
        let mut glyphs = Vec::new();
        let mut template = Row::new(i, offset + chars.len());
        let mut lead_extra = 0.0;
        let (hidden, look) = match &found {
            None => (0, PLAIN),
            Some(p) => {
                let depth = p.indent / 2;
                let pad = " ".repeat(p.indent);
                let look = match &p.marker {
                    Marker::Heading(level) => {
                        template.heading = *level as u8;
                        template.tall = proportional && *level <= 2;
                        Look {
                            tone: Tone::Heading,
                            font: Font {
                                bold: true,
                                size: HEADINGS[level - 1],
                                ..Font::default()
                            },
                            underline: false,
                        }
                    }
                    Marker::Quote => {
                        if proportional {
                            template.quote = true;
                            lead_extra = 1.4;
                        } else {
                            glyphs.extend(decoration("▌ ", Tone::Dim));
                        }
                        PLAIN
                    }
                    Marker::Bullet => {
                        let bullet = ["•", "◦", "▪"][depth % 3];
                        glyphs.extend(decoration(&format!("{pad}{bullet} "), Tone::Text));
                        PLAIN
                    }
                    Marker::Ordered(number) => {
                        glyphs.extend(decoration(&format!("{pad}{number}. "), Tone::Dim));
                        PLAIN
                    }
                    Marker::Task { checked, mark } => {
                        template.checkbox = Some((p.indent as f32, offset + mark));
                        let symbol = if *checked { "☑" } else { "☐" };
                        glyphs.extend(decoration(&pad, Tone::Text));
                        glyphs.extend(decoration(symbol, Tone::Check(*checked)));
                        glyphs.extend(decoration(" ", Tone::Text));
                        PLAIN
                    }
                };
                (p.hidden, look)
            }
        };
        let decorated = glyphs.len();
        inline(
            &chars[hidden..],
            offset + hidden,
            look,
            &context,
            &mut glyphs,
        );
        // A paragraph runs on across its source lines, to fill the page. Each line break
        // shows as a space that still stands for the newline, so the file doesn't change.
        let flows = !chars.is_empty()
            && !matches!(
                found.as_ref().map(|p| &p.marker),
                Some(Marker::Heading(_) | Marker::Quote)
            );
        let mut last = i;
        while flows && continues_paragraph(&lines, last) {
            let newline = lines[last].0 + lines[last].1.chars().count();
            glyphs.extend(decoration(" ", Tone::Text));
            if let Some(space) = glyphs.last_mut() {
                space.at = Some(newline);
            }
            last += 1;
            let next: Vec<char> = lines[last].1.chars().collect();
            let indent = next.iter().take_while(|c| c.is_whitespace()).count();
            let start = lines[last].0 + indent;
            inline(&next[indent..], start, look, &context, &mut glyphs);
        }
        template.end = lines[last].0 + lines[last].1.chars().count();
        measure_glyphs(&mut glyphs, measure);
        // Checkboxes get a little more room than a glyph, for the box drawn there.
        if proportional {
            for g in &mut glyphs[..decorated] {
                if matches!(g.tone, Tone::Check(_)) {
                    g.advance = 1.3;
                }
            }
        }
        // List text lines up whatever the marker's width, as in a real document.
        if proportional && decorated > 0 {
            let pad = found.as_ref().map_or(0, |p| p.indent) as f32;
            let width: f32 = glyphs[..decorated].iter().map(|g| g.advance).sum();
            glyphs[decorated - 1].advance += (pad + 1.8 - width).max(0.0);
            glyphs[decorated - 1].tone = Tone::TableLine;
        }
        let lead = glyphs[..decorated].iter().map(|g| g.advance).sum::<f32>() + lead_extra;
        if lead_extra > 0.0 {
            // The quote's bar takes the indent, so the text starts after it.
            let mut indent = decoration(" ", Tone::TableLine);
            indent[0].advance = lead_extra;
            indent.append(&mut glyphs);
            glyphs = indent;
        }
        let tall = template.tall;
        let first = rows.len();
        wrap(template, glyphs, lead, width, &mut rows);
        // Each row belongs to the source line it starts in, for comments and change marks.
        for row in &mut rows[first..] {
            if let Some(at) = row.glyphs.iter().find_map(|g| g.at) {
                row.line = lines
                    .partition_point(|(start, _)| *start <= at)
                    .saturating_sub(1);
            }
        }
        if tall {
            rows.push(spacer(i, offset));
        }
        i = last + 1;
    }
    rows
}

/// The row and column of a source position. Positions inside hidden text land on the
/// nearest visible place.
pub fn locate(rows: &[Row], pos: usize) -> (usize, f32) {
    for (index, row) in rows.iter().enumerate() {
        let inside = if row.last {
            pos <= row.end
        } else {
            pos < row.end
        };
        if row.stop && inside {
            return (index, row.caret_x(pos));
        }
    }
    let last = rows.iter().rposition(|r| r.stop).unwrap_or(0);
    (last, rows.get(last).map_or(0.0, |r| r.caret_x(usize::MAX)))
}

/// Every place the caret can rest, in source order.
fn stops(rows: &[Row]) -> Vec<usize> {
    let mut all: Vec<usize> = rows
        .iter()
        .filter(|r| r.stop)
        .flat_map(|r| {
            let ends = r
                .glyphs
                .windows(2)
                .filter_map(|w| match (w[0].at, w[1].at) {
                    (Some(at), None) => Some(at + 1),
                    _ => None,
                });
            let starts = r.glyphs.iter().filter_map(|g| g.at);
            let empty = (0..r.cells.len())
                .filter(|index| r.cell_is_empty(*index))
                .map(|index| r.cells[index]);
            starts
                .chain(ends)
                .chain(empty)
                .chain([r.end])
                .collect::<Vec<_>>()
        })
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

pub fn snap(rows: &[Row], pos: usize) -> usize {
    let (row, x) = locate(rows, pos);
    let Some(row) = rows.get(row) else {
        return pos;
    };
    // Right before a closed formula is a real place, though no glyph starts there.
    if row
        .glyphs
        .iter()
        .any(|g| matches!(g.tone, Tone::Formula { .. }) && g.at == Some(pos + 1))
    {
        return pos;
    }
    row.position_at(x)
}

pub fn step(rows: &[Row], pos: usize, forward: bool) -> usize {
    let all = stops(rows);
    let index = all.partition_point(|s| *s < pos);
    if forward {
        let next = if all.get(index) == Some(&pos) {
            index + 1
        } else {
            index
        };
        all.get(next).copied().unwrap_or(pos)
    } else {
        index.checked_sub(1).map_or(pos, |i| all[i])
    }
}

/// Moves `delta` caret rows, keeping to column `goal`.
pub fn vertical(rows: &[Row], pos: usize, delta: isize, goal: f32) -> usize {
    let (mut row, _) = locate(rows, pos);
    for _ in 0..delta.unsigned_abs() {
        let next = if delta < 0 {
            rows[..row].iter().rposition(|r| r.stop)
        } else {
            rows.iter()
                .skip(row + 1)
                .position(|r| r.stop)
                .map(|i| row + 1 + i)
        };
        match next {
            Some(found) => row = found,
            None => break,
        }
    }
    rows.get(row).map_or(pos, |r| r.position_at(goal))
}

pub fn row_edge(rows: &[Row], pos: usize, end: bool) -> usize {
    let (row, _) = locate(rows, pos);
    let Some(row) = rows.get(row) else {
        return pos;
    };
    if end {
        row.end
    } else {
        row.glyphs.iter().find_map(|g| g.at).unwrap_or(row.end)
    }
}

/// The next table cell after `pos`, for Tab.
pub fn next_cell(rows: &[Row], pos: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    let here = rows.get(row)?;
    if here.cells.is_empty() {
        return None;
    }
    let mut after = rows[row..]
        .iter()
        .take_while(|r| !r.cells.is_empty() || !r.stop)
        .flat_map(|r| r.cells.iter().copied());
    after.find(|at| *at > pos)
}

/// What Enter should do on a line, and Backspace at its start.
pub enum Intent {
    /// Start the next line with this marker.
    Continue(String),
    /// The item is empty: remove its marker, which ends the list.
    EndList(usize),
    Plain,
}

/// Enter at the end of a list item carries the list on.
pub fn enter_intent(line: &str) -> Intent {
    let Some(p) = prefix(line) else {
        return Intent::Plain;
    };
    if matches!(p.marker, Marker::Heading(_)) {
        return Intent::Plain;
    }
    if line.chars().count() == p.hidden {
        return Intent::EndList(p.hidden);
    }
    let pad = " ".repeat(p.indent);
    let chars: Vec<char> = line.chars().collect();
    Intent::Continue(match p.marker {
        Marker::Ordered(number) => format!("{pad}{}. ", number + 1),
        Marker::Task { .. } => format!("{pad}- [ ] "),
        _ => chars[..p.hidden].iter().collect(),
    })
}

/// The web address at a char position on a line: a `[text](url)` link or a bare URL.
pub fn link_at(line: &str, column: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some(close) = chars[i..].iter().position(|c| *c == ']')
            && chars.get(i + close + 1) == Some(&'(')
            && let Some(end) = chars[i + close..].iter().position(|c| *c == ')')
        {
            let end = i + close + end;
            if (i..=end).contains(&column) {
                return Some(chars[i + close + 2..end].iter().collect());
            }
            i = end;
        }
        i += 1;
    }
    let mut start = 0;
    for word in line.split(' ') {
        let length = word.chars().count();
        if (start..=start + length).contains(&column)
            && (word.starts_with("http://") || word.starts_with("https://"))
        {
            return Some(word.trim_end_matches(['.', ',', ')', ';']).to_string());
        }
        start += length + 1;
    }
    None
}

/// Length of the hidden marker, which Backspace removes as a unit.
pub fn hidden_prefix(line: &str) -> usize {
    prefix(line).map_or(0, |p| p.hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_render::{CellMeasure, FontMeasure};

    fn text(row: &Row) -> String {
        row.glyphs.iter().map(|g| g.ch).collect()
    }

    fn page(source: &str, width: usize) -> Vec<Row> {
        layout(source, width, &CellMeasure, &Registry::default())
    }

    fn texts(source: &str) -> Vec<String> {
        page(source, 40).iter().map(text).collect()
    }

    #[test]
    fn block_markers_are_hidden() {
        assert_eq!(texts("# Title"), ["Title"]);
        assert_eq!(texts("- item"), ["• item"]);
        assert_eq!(texts("  - inner"), ["  ◦ inner"]);
        assert_eq!(texts("3. third"), ["3. third"]);
        assert_eq!(texts("> said"), ["▌ said"]);
        assert_eq!(texts("- [ ] todo"), ["☐ todo"]);
        assert_eq!(texts("- [x] done"), ["☑ done"]);
    }

    #[test]
    fn inline_markers_are_hidden() {
        assert_eq!(texts("a **b** *c* `d` e"), ["a b c d e"]);
        assert_eq!(
            texts("see [the docs](http://x.y) now"),
            ["see the docs now"]
        );
    }

    #[test]
    fn unmatched_markers_stay_as_text() {
        assert_eq!(
            texts("2 * 3 and snake_case_name"),
            ["2 * 3 and snake_case_name"]
        );
        assert_eq!(texts("**open"), ["**open"]);
    }

    #[test]
    fn every_glyph_knows_its_source_position() {
        let source = "# Hi **yo**";
        let rows = page(source, 40);
        let chars: Vec<char> = source.chars().collect();
        for g in &rows[0].glyphs {
            assert_eq!(chars[g.at.unwrap()], g.ch);
        }
    }

    #[test]
    fn fences_hide_and_the_language_is_a_label() {
        assert_eq!(
            texts("```rust\nlet x;\n```\nafter"),
            ["rust", "let x;", "after"]
        );
        let rows = page("```rust\nlet x;\n```", 40);
        assert!(!rows[0].stop);
        assert!(rows[1].shaded);
        assert_eq!(rows[1].glyphs[0].tone, Tone::Block(Some(Token::Keyword)));
    }

    #[test]
    fn tables_align_with_pipes_and_rule_hidden() {
        let rows = page(
            "| Path | Holds |
|---|---|
| a | long text |",
            40,
        );
        assert_eq!(rows.len(), 2, "the rule isn't a row");
        assert!(!text(&rows[1]).contains('|'));
        let x = |r: &Row, c: char| r.glyphs.iter().find(|g| g.ch == c).unwrap().x;
        assert_eq!(x(&rows[0], 'H'), x(&rows[1], 'l'), "columns line up");
        assert!(rows[0].table.as_ref().unwrap().header);
    }

    #[test]
    fn front_matter_delimiters_hide() {
        assert_eq!(texts("---\ntitle: x\n---\nbody"), ["title: x", "body"]);
    }

    #[test]
    fn long_lines_wrap_under_their_text() {
        let rows = page("- one two three four", 12);
        assert_eq!(text(&rows[0]), "• one two ");
        assert_eq!(text(&rows[1]), "three four");
        assert_eq!(
            rows[1].glyphs[0].x, 2.0,
            "continuations line up under the text"
        );
        assert!(!rows[0].last);
        assert!(rows[1].last);
    }

    #[test]
    fn the_caret_skips_hidden_markers() {
        let rows = page(
            "# Hi
next",
            40,
        );
        assert_eq!(snap(&rows, 0), 2);
        assert_eq!(step(&rows, 2, false), 2);
        assert_eq!(step(&rows, 4, true), 5);
        assert_eq!(locate(&rows, 2), (0, 0.0));
    }

    #[test]
    fn headings_stand_taller_than_body_text() {
        let m = FontMeasure::new(17, 37);
        let rows = layout(
            "# Big

body",
            60,
            &m,
            &Registry::default(),
        );
        let big = rows.iter().find(|r| r.heading == 1).unwrap();
        assert!(big.tall);
        assert!(big.glyphs[0].font.size > 150);
        assert!(!big.glyphs[0].font.mono, "body text is proportional");
    }

    #[test]
    fn wrapping_uses_real_widths() {
        let m = FontMeasure::new(17, 37);
        let thin = layout(&"i".repeat(100), 40, &m, &Registry::default());
        let wide = layout(&"W".repeat(100), 40, &m, &Registry::default());
        assert!(thin.len() < wide.len());
    }

    fn page_rows(text: &str, width: usize) -> Vec<Row> {
        layout(text, width, &CellMeasure, &Registry::default())
    }

    fn row_text(row: &Row) -> String {
        row.glyphs.iter().map(|g| g.ch).collect()
    }

    #[test]
    fn a_paragraph_runs_on_across_source_lines() {
        let rows = page_rows("one two\nthree four\n\nnext\n", 60);
        assert_eq!(row_text(&rows[0]), "one two three four");
        assert_eq!(row_text(&rows[2]), "next");
    }

    #[test]
    fn the_space_at_a_join_stands_for_the_newline() {
        let rows = page_rows("ab\ncd\n", 60);
        let space = rows[0].glyphs.iter().find(|g| g.ch == ' ').unwrap();
        assert_eq!(space.at, Some(2), "the newline's position");
        let c = rows[0].glyphs.iter().find(|g| g.ch == 'c').unwrap();
        assert_eq!(c.at, Some(3));
        assert_eq!(rows[0].end, 5);
    }

    #[test]
    fn structure_stops_a_paragraph() {
        for next in ["# Heading", "- item", "1. item", "> quote", "```", "---"] {
            let text = format!("para\n{next}\n");
            let rows = page_rows(&text, 60);
            assert_eq!(row_text(&rows[0]), "para", "{next}");
        }
    }

    #[test]
    fn a_hard_break_is_kept() {
        assert_eq!(
            row_text(&page_rows("one  \ntwo\n", 60)[0]).trim_end(),
            "one"
        );
    }

    #[test]
    fn a_list_item_takes_its_continuation_line() {
        let rows = page_rows("- one\n  two\n- three\n", 60);
        assert!(
            row_text(&rows[0]).ends_with("one two"),
            "{:?}",
            row_text(&rows[0])
        );
    }

    #[test]
    fn a_wrapped_row_knows_its_source_line() {
        let long = "word ".repeat(10);
        let text = format!("{}\n{}\n", long.trim_end(), long.trim_end());
        let rows = page_rows(&text, 30);
        assert_eq!(rows[0].line, 0);
        let last = rows.iter().rfind(|r| !r.glyphs.is_empty()).unwrap();
        assert_eq!(last.line, 1);
    }

    #[test]
    fn running_text_uses_the_width_it_is_given() {
        let m = FontMeasure::new(17, 37);
        let rows = layout(&"word ".repeat(60), 150, &m, &Registry::default());
        let widest = rows
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| g.x + g.advance))
            .fold(0.0, f32::max);
        assert!(widest > 120.0 && widest <= 150.5, "{widest}");
    }

    #[test]
    fn clicks_land_between_real_glyphs() {
        let m = FontMeasure::new(17, 37);
        let rows = layout("iiiiWWWW", 60, &m, &Registry::default());
        let row = rows
            .iter()
            .find(|r| r.stop && !r.glyphs.is_empty())
            .unwrap();
        assert_eq!(row.position_at(row.glyphs[5].x + 0.05), 5);
    }

    #[test]
    fn vertical_moves_keep_the_column_and_skip_rules() {
        let rows = page("| a |\n|---|\n| b |", 40);
        let down = vertical(&rows, 2, 1, 0.0);
        assert_eq!(rows[locate(&rows, down).0].line, 2);
    }

    #[test]
    fn padded_tables_show_every_column() {
        let rows = texts("| A   | B   |\n| --- | --- |\n| one | two |\n|     |     |\n");
        assert!(rows[1].contains("two"), "{rows:?}");
    }

    #[test]
    fn tab_finds_the_next_cell() {
        let source = "| a | b |\n|---|---|\n| c | d |";
        let rows = page(source, 40);
        let chars: Vec<char> = source.chars().collect();
        let first = next_cell(&rows, 2).unwrap();
        assert_eq!(chars[first], 'b');
        let second = next_cell(&rows, first).unwrap();
        assert_eq!(chars[second], 'c');
    }

    #[test]
    fn links_are_found_under_the_cursor() {
        let line = "see [docs](http://a.b/c) and https://x.y/z.";
        assert_eq!(link_at(line, 6).as_deref(), Some("http://a.b/c"));
        assert_eq!(link_at(line, 32).as_deref(), Some("https://x.y/z"));
        assert_eq!(link_at(line, 1), None);
    }

    type Typeset = (Vec<Row>, Vec<Figure>, Vec<(usize, usize)>);

    /// Lays a page out as the pixel tiers do, with `open` showing as source.
    fn typeset(text: &str, open: Option<(usize, usize)>) -> Typeset {
        let m = FontMeasure::new(17, 37);
        let extras = Extras {
            open,
            ..Extras::default()
        };
        let rows = layout_with(text, 60, &m, &Registry::default(), &extras);
        (rows, extras.figures.into_inner(), extras.spans.into_inner())
    }

    fn formulas(rows: &[Row]) -> usize {
        let is = |g: &&Glyph| matches!(g.tone, Tone::Formula { .. });
        rows.iter().flat_map(|r| r.glyphs.iter().filter(is)).count()
    }

    #[test]
    fn formulas_show_as_pictures_in_the_line() {
        let source = "Energy is $E = mc^2$ here.";
        let (rows, figures, spans) = typeset(source, None);
        assert_eq!(formulas(&rows), 1);
        assert_eq!(figures.len(), 1);
        assert_eq!(spans, [(10, 20)]);
        let shown: String = rows.iter().map(text).collect();
        assert!(!shown.contains("mc^2"), "{shown}");
        assert!(shown.contains("here."));
    }

    #[test]
    fn a_formula_shows_its_source_when_open() {
        let source = "Energy is $E = mc^2$ here.";
        let (rows, figures, _) = typeset(source, Some((10, 20)));
        assert_eq!(formulas(&rows), 0);
        assert_eq!(figures.len(), 1, "a preview, under the line being typed");
        let shown: String = rows.iter().map(text).collect();
        assert!(shown.contains("$E = mc^2$"), "{shown}");
    }

    #[test]
    fn prices_are_not_formulas() {
        let (rows, _, spans) = typeset("It costs $5 and then $10 more.", None);
        assert_eq!(formulas(&rows), 0);
        assert!(spans.is_empty());
    }

    #[test]
    fn a_block_formula_takes_rows_of_its_own() {
        let source = "before\n\n$$\n\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}\n$$\n\nafter\n";
        let (rows, figures, spans) = typeset(source, None);
        assert_eq!(figures.len(), 1);
        assert_eq!(spans.len(), 1);
        let at = rows.iter().position(|r| r.figure.is_some()).unwrap();
        assert!(rows[at].stop, "the caret can reach it");
        assert!(!rows[at + 1].stop, "and the rest is room for it");
        let shown: String = rows.iter().map(text).collect();
        assert!(!shown.contains("sum"), "{shown}");
        let (open, figures, _) = typeset(source, Some(spans[0]));
        let shown: String = open.iter().map(text).collect();
        assert!(shown.contains("\\sum"), "source to edit: {shown}");
        assert_eq!(figures.len(), 1, "with the result still under it");
    }

    #[test]
    fn a_formula_that_wont_typeset_stays_readable() {
        let (rows, figures, _) = typeset("Bad $\\frac{$ here", None);
        assert!(figures.is_empty());
        let shown: String = rows.iter().map(text).collect();
        assert!(shown.contains("\\frac{"), "{shown}");
    }

    #[test]
    fn the_cell_tier_shows_formulas_as_text() {
        assert_eq!(texts("Is $x^2$ big"), ["Is x^2 big"]);
    }

    #[test]
    fn images_show_in_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let pixels = image::RgbaImage::from_pixel(340, 74, image::Rgba([200, 30, 30, 255]));
        pixels.save(dir.path().join("plot.png")).unwrap();
        let source = "![The plot](plot.png)\n\nafter\n";
        let m = FontMeasure::new(17, 37);
        let extras = Extras {
            folder: Some(dir.path()),
            ..Extras::default()
        };
        let rows = layout_with(source, 60, &m, &Registry::default(), &extras);
        let figures = extras.figures.into_inner();
        assert_eq!(figures.len(), 1);
        assert_eq!(figures[0].width, 20.0);
        assert_eq!(figures[0].height, 2.0);
        let caption = rows.iter().find(|r| text(r).contains("The plot")).unwrap();
        assert!(!caption.stop);
        assert!(rows.iter().any(|r| r.figure.is_some()));
        let missing = typeset("![gone](nothing.png)\n", None);
        assert!(missing.1.is_empty(), "a missing image stays as its source");
    }

    #[test]
    fn enter_carries_lists_on_and_empty_items_end_them() {
        assert!(matches!(enter_intent("- a"), Intent::Continue(s) if s == "- "));
        assert!(matches!(enter_intent("  2. a"), Intent::Continue(s) if s == "  3. "));
        assert!(matches!(enter_intent("- [x] a"), Intent::Continue(s) if s == "- [ ] "));
        assert!(matches!(enter_intent("> a"), Intent::Continue(s) if s == "> "));
        assert!(matches!(enter_intent("- "), Intent::EndList(2)));
        assert!(matches!(enter_intent("# a"), Intent::Plain));
        assert!(matches!(enter_intent("plain"), Intent::Plain));
    }
}
