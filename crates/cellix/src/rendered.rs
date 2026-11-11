//! Markdown as one editable page, the way Xcode's rendered editor works. The source stays
//! the document: this lays it out with its markers hidden, and every visible character
//! remembers its source position, so clicks and edits land in the right place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use cellix_doc::diagram::{Kind, Raster};
use cellix_doc::figure::image_line;
use cellix_doc::latex::Project;
use cellix_doc::notebook::{self, Notebook, Output};
use cellix_lang::{Registry, Token};
use cellix_render::{Bitmap, Font, INTERFACE_SIZE, Measure, Pixels};

use crate::snippets::{Snippet, State};

mod data;
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
    /// Text with a line through it.
    Struck,
    /// Text behind a highlighter.
    Marked,
    /// Text set small and raised, or small and lowered.
    Raised,
    Lowered,
    /// A footnote's number where it's cited.
    Footnote,
    /// A callout's title, by its kind.
    Callout(Callout),
    /// Text a tracked change adds, and text it takes away.
    Inserted,
    Deleted,
    /// What failed, in the colour of failure.
    Failed,
    /// An equation's number, moved this many 64ths of a row from the middle of its row.
    Margin(i8),
}

/// The kinds of GitHub's callouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callout {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Callout {
    /// The kind a quote's first line names, as `[!NOTE]`, with the title written after it, as
    /// in Obsidian's `[!tip]- Keep it short`, if there is one.
    fn named(line: &str) -> Option<(Self, Option<String>)> {
        let (kind, title) = cellix_doc::html::callout(line)?;
        let kind = match kind {
            "NOTE" => Self::Note,
            "TIP" => Self::Tip,
            "IMPORTANT" => Self::Important,
            "WARNING" => Self::Warning,
            _ => Self::Caution,
        };
        Some((kind, title))
    }

    fn title(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }
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
    /// Which row of its source line this is. A cell whose text doesn't fit its column runs
    /// on under itself, so a line of the table can take several rows of the page.
    pub part: usize,
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
    /// The kind of callout this row is part of, which colours its bar.
    pub callout: Option<Callout>,
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
            callout: None,
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
    /// Diagrams by what they're drawn from, with why one couldn't be.
    diagrams: HashMap<(Kind, String, u32), Result<Figure, String>>,
    /// Pieces of LaTeX that have been built, by their key and the room they had.
    built: HashMap<(u64, u32), Option<Figure>>,
    /// Where each row of a display stands, by its source.
    rows: HashMap<String, Option<Vec<f32>>>,
}

/// How a piece of LaTeX stands on the page.
pub(crate) enum Built {
    Shown(usize),
    /// LaTeX is on it.
    Waiting,
    Failed(String),
    /// Nothing here can build it.
    Never,
}

/// What a page is written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dialect {
    #[default]
    Markdown,
    Latex,
    /// Rows of fields, split by this.
    Data(char),
}

impl Dialect {
    /// The dialect of a file, if it opens as a page.
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "md" | "markdown" | "mdx" | "ipynb" => Some(Self::Markdown),
            "tex" | "ltx" => Some(Self::Latex),
            "csv" => Some(Self::Data(',')),
            "tsv" => Some(Self::Data('\t')),
            _ => None,
        }
    }

    /// Whether this is writing, with a toolbar, comments on prose and the like.
    pub fn is_writing(self) -> bool {
        !matches!(self, Self::Data(_))
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
    /// Diagrams are drawn for the sheet they stand on.
    pub dark: bool,
    pub library: Option<&'a RefCell<Library>>,
    /// Diagrams standing on their last good picture, and why the current source won't draw.
    pub broken: RefCell<Vec<(usize, String)>>,
    /// Where built pieces of LaTeX are kept. Without it they aren't built.
    pub built: Option<&'a Path>,
    /// Pieces of LaTeX this page is waiting for.
    pub wanted: RefCell<Vec<Snippet>>,
    /// Footnotes in the order they're first met.
    pub footnotes: RefCell<Vec<String>>,
    /// The notebook this page is, for what its cells printed.
    pub notebook: Option<&'a Notebook>,
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

/// Turns black lines on white paper into ink, so a drawing takes the sheet's colours. One with
/// colours of its own stays as it is, on its paper.
fn figure_of_built(page: &cellix_doc::pdf::Page, key: u64, cell: (u32, u32)) -> Figure {
    let grey = page
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[0].abs_diff(p[1]) < 8 && p[1].abs_diff(p[2]) < 8);
    let (kind, pixels) = if grey {
        let ink = page.rgba.as_chunks::<4>().0.iter().map(|p| 255 - p[1]);
        (Pixels::Coverage, Arc::new(ink.collect::<Vec<u8>>()))
    } else {
        (Pixels::Rgba, page.rgba.clone())
    };
    Figure {
        bitmap: Bitmap {
            id: key ^ u64::from(page.width),
            width: page.width,
            height: page.height,
            kind,
            pixels,
        },
        width: page.width as f32 / cell.0 as f32,
        height: page.height as f32 / cell.1 as f32,
        baseline: 0.0,
    }
}

/// Pixels from a drawing, with an id that stands for what they show.
fn figure_of_raster(raster: Raster, identity: impl Hash, cell: (u32, u32)) -> Figure {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity.hash(&mut hasher);
    (raster.width, raster.height).hash(&mut hasher);
    Figure {
        bitmap: Bitmap {
            id: hasher.finish(),
            width: raster.width,
            height: raster.height,
            kind: Pixels::Rgba,
            pixels: Arc::new(raster.rgba),
        },
        width: raster.width as f32 / cell.0 as f32,
        height: raster.height as f32 / cell.1 as f32,
        baseline: 0.0,
    }
}

/// How a picture takes up the page.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fit {
    /// Its own size, but never past this many cells.
    AtMost(f32),
    /// Exactly this many cells across, because the document says so.
    Exactly(f32),
}

impl Fit {
    fn cells(self) -> f32 {
        match self {
            Self::AtMost(cells) | Self::Exactly(cells) => cells,
        }
    }
}

/// Reads an image and sets it to fit the page, keeping its shape.
fn load_image(path: &Path, cell: (u32, u32), fit: Fit) -> Option<Figure> {
    let wanted = (fit.cells() * cell.0 as f32) as u32;
    if path.extension().is_some_and(|e| e == "svg") {
        let text = std::fs::read_to_string(path).ok()?;
        let raster = cellix_doc::diagram::rasterize(&text, device_scale(cell), wanted).ok()?;
        return Some(sized(figure_of_raster(raster, (path, wanted), cell), fit));
    }
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
    let figure = Figure {
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
    };
    Some(sized(figure, fit))
}

/// Stands a figure at the width the document asks for, keeping its shape.
fn sized(mut figure: Figure, fit: Fit) -> Figure {
    if let Fit::Exactly(cells) = fit {
        figure.height *= cells / figure.width.max(0.01);
        figure.width = cells;
    }
    figure
}

/// Device pixels to a drawing's own pixels. A row is about eighteen of those.
fn device_scale(cell: (u32, u32)) -> f32 {
    (cell.1 as f32 / 18.0).max(1.0)
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
        // Inside a suggestion that started or ends in another block.
        if let Some(shown) = context.across(at) {
            if let Some(tone) = shown {
                out.push(glyph(chars[i], Some(at), Look { tone, ..look }));
            }
            i += 1;
            continue;
        }
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
        let toned = |tone: Tone, size: Option<u8>| Look {
            tone,
            font: Font {
                size: size.unwrap_or(look.font.size),
                ..look.font
            },
            ..look
        };
        // A word's own tilde or caret, like `x~y z~`, is left alone: these hug their text.
        let tight = |marker: char| {
            rest[0] == marker
                && (2..rest.len().min(32))
                    .find(|n| rest[*n] == marker)
                    .is_some_and(|n| rest[1..n].iter().all(|c| !c.is_whitespace()))
        };
        let strong = Look {
            font: Font {
                bold: true,
                italic: true,
                ..look.font
            },
            ..look
        };
        let used = if rest.starts_with(&['*', '*', '*']) {
            wrap(&['*', '*', '*'], strong, out)
        } else if rest.starts_with(&['{', '+', '+']) {
            critic(
                rest,
                &['+', '+', '}'],
                toned(Tone::Inserted, None),
                at,
                context,
                out,
            )
        } else if rest.starts_with(&['{', '-', '-']) {
            critic(
                rest,
                &['-', '-', '}'],
                toned(Tone::Deleted, None),
                at,
                context,
                out,
            )
        } else if rest.starts_with(&['{', '=', '=']) {
            critic(
                rest,
                &['=', '=', '}'],
                toned(Tone::Marked, None),
                at,
                context,
                out,
            )
        } else if rest.starts_with(&['{', '>', '>']) {
            critic(
                rest,
                &['<', '<', '}'],
                toned(Tone::Dim, Some(85)),
                at,
                context,
                out,
            )
        } else if rest.starts_with(&['{', '~', '~']) {
            replacement(rest, look, at, context, out)
        } else if rest.starts_with(&['~', '~']) {
            wrap(&['~', '~'], toned(Tone::Struck, None), out)
        } else if rest.starts_with(&['=', '=']) && rest.get(2).is_some_and(|c| *c != ' ') {
            wrap(&['=', '='], toned(Tone::Marked, None), out)
        } else if tight('~') {
            wrap(&['~'], toned(Tone::Lowered, Some(72)), out)
        } else if tight('^') {
            wrap(&['^'], toned(Tone::Raised, Some(72)), out)
        } else if rest.starts_with(&['*', '*']) {
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
        } else if rest.starts_with(&['[', '^']) {
            footnote(rest, at, look, context, out)
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

/// A tracked change or highlight of the form `{++text++}`, with its markers hidden.
fn critic(
    rest: &[char],
    close: &[char],
    look: Look,
    at: usize,
    context: &Context,
    out: &mut Vec<Glyph>,
) -> Option<usize> {
    let end = (3..rest.len()).find(|i| rest[*i..].starts_with(close))?;
    inline(&rest[3..end], at + 3, look, context, out);
    Some(end + close.len())
}

/// `{~~old~>new~~}`: what's going, then what's coming.
fn replacement(
    rest: &[char],
    look: Look,
    at: usize,
    context: &Context,
    out: &mut Vec<Glyph>,
) -> Option<usize> {
    let arrow = (3..rest.len()).find(|i| rest[*i..].starts_with(&['~', '>']))?;
    let end = (arrow..rest.len()).find(|i| rest[*i..].starts_with(&['~', '~', '}']))?;
    let tone = |tone| Look { tone, ..look };
    inline(&rest[3..arrow], at + 3, tone(Tone::Deleted), context, out);
    inline(
        &rest[arrow + 2..end],
        at + arrow + 2,
        tone(Tone::Inserted),
        context,
        out,
    );
    Some(end + 3)
}

/// `[^id]`: the footnote's number, counted in the order they're cited.
fn footnote(
    rest: &[char],
    at: usize,
    look: Look,
    context: &Context,
    out: &mut Vec<Glyph>,
) -> Option<usize> {
    let close = rest.iter().position(|c| *c == ']')?;
    let id: String = rest[2..close].iter().collect();
    if id.is_empty() || id.contains(' ') || rest.get(close + 1) == Some(&':') {
        return None;
    }
    let number = context.footnote_number(&id);
    let shown = Look {
        tone: Tone::Footnote,
        font: Font {
            size: 72,
            ..look.font
        },
        ..look
    };
    for (n, c) in number.to_string().chars().enumerate() {
        out.push(glyph(
            c,
            Some(at + 2 + n.min(id.chars().count() - 1)),
            shown,
        ));
    }
    Some(close + 1)
}

/// A footnote's text: `[^id]: words`, with where the words start.
fn footnote_definition(line: &str) -> Option<(String, usize)> {
    let rest = line.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let id = &rest[..close];
    (!id.is_empty() && !id.contains(' ')).then(|| {
        let after = &rest[close + 2..];
        let spaces = after.chars().take_while(|c| *c == ' ').count();
        (id.to_string(), id.chars().count() + 4 + spaces)
    })
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
        // A space that doesn't fit stays at its row's end, past the edge, where it doesn't
        // show: at the start of the next row it would set that row in by a space.
        let hangs = g.ch == ' ' && g.at.is_some();
        if x + g.advance > width && !current.is_empty() && !hangs {
            let cut = current
                .iter()
                .rposition(|g| g.ch == ' ' && g.at.is_some())
                .filter(|i| current[*i].x > lead)
                .map_or_else(|| after_slash(&current), |i| i + 1);
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

/// Where a row with no space in it is best broken: after a slash near its end, as a link or
/// a path has, or else at its end.
fn after_slash(row: &[Glyph]) -> usize {
    /// How far back from the end a slash is still worth breaking at.
    const NEAR: usize = 8;
    let from = row.len().saturating_sub(NEAR);
    let slash = row[from..].iter().rposition(|g| g.ch == '/');
    slash.map_or(row.len(), |at| from + at + 1)
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

/// Lays out a table with its pipes and rule hidden: real columns, with padding, no wider
/// together than `width` when their words allow it.
fn table(
    lines: &[(usize, &str)],
    first: usize,
    width: f32,
    context: &Context,
    rows: &mut Vec<Row>,
) {
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
    let ruled = context.measure.in_cells();
    place_table(parsed, starts, &places, (&aligns, width, ruled), rows);
}

/// Space between a table cell's edge and its text, in cells.
const CELL_PADDING: f32 = 1.0;
/// The least a table's column holds, in cells of text.
const NARROWEST_COLUMN: f32 = 3.0;

/// How wide each column of a table is, text only, for a table `width` wide in all. Each
/// gets what its longest cell needs when they all fit. When they don't, each keeps room for
/// its longest word and they share what's left by how much more they wanted, so cells wrap
/// rather than the table running off the page. A word longer than a column's even share of
/// the page, as a link can be, is broken rather than widen the table for every row, though
/// never an ordinary word: a table with more columns than the page has room for is left
/// wider than it, to be moved sideways, sooner than have its numbers broken in two.
fn column_widths(parsed: &[Vec<Vec<Glyph>>], count: usize, width: f32) -> Vec<f32> {
    let cells = |c: usize| parsed.iter().filter_map(move |row| row.get(c));
    let wanted: Vec<f32> = (0..count)
        .map(|c| {
            let widths = cells(c).map(|cell| cell.iter().map(|g| g.advance).sum::<f32>());
            widths.fold(NARROWEST_COLUMN, f32::max).ceil()
        })
        .collect();
    let room = width - count as f32 * CELL_PADDING * 2.0;
    if wanted.iter().sum::<f32>() <= room {
        return wanted;
    }
    /// The longest word a column keeps whole, however many columns share the page.
    const WHOLE_WORD: f32 = 16.0;
    let even = (room / count.max(1) as f32).floor().max(WHOLE_WORD);
    let least: Vec<f32> = (0..count)
        .map(|c| {
            let words = cells(c).flat_map(|cell| cell.split(|g| g.ch == ' ' && g.at.is_some()));
            let widths = words.map(|word| word.iter().map(|g| g.advance).sum::<f32>());
            let longest = widths.fold(NARROWEST_COLUMN, f32::max).ceil();
            longest.min(even).min(wanted[c])
        })
        .collect();
    let (least_all, wanted_all) = (least.iter().sum::<f32>(), wanted.iter().sum::<f32>());
    if least_all >= room {
        return least;
    }
    let share = (room - least_all) / (wanted_all - least_all);
    let shared = least.iter().zip(&wanted);
    let mut widths: Vec<f32> = shared
        .map(|(least, wanted)| (least + (wanted - least) * share).floor())
        .collect();
    // What rounding down left over goes to the column that gave up most, so the table ends
    // where the text around it does.
    let spare = (room - widths.iter().sum::<f32>()).floor().max(0.0);
    let gave = |c: &usize| wanted[*c] - widths[*c];
    if let Some(most) = (0..count).max_by(|a, b| gave(a).total_cmp(&gave(b))) {
        widths[most] += spare;
    }
    widths
}

/// Sets a table's cells in columns, no wider together than `width` when their words allow
/// it: a cell with more than fits its column runs on under itself, and its line of the
/// table takes as many rows as its longest cell. Each line has its place in the source and
/// where that line ends.
fn place_table(
    parsed: Vec<Vec<Vec<Glyph>>>,
    starts: Vec<Vec<usize>>,
    places: &[(usize, usize)],
    (aligns, width, ruled): (&[char], f32, bool),
    rows: &mut Vec<Row>,
) {
    let count = parsed.iter().map(Vec::len).max().unwrap_or(0);
    let widths = column_widths(&parsed, count, width);
    let mut columns = vec![0.0f32];
    for c in 0..count {
        columns.push(columns[c] + widths[c] + CELL_PADDING * 2.0);
    }
    let shift = |c: usize, room: f32| match aligns.get(c) {
        Some('r') => room,
        Some('c') => room / 2.0,
        _ => 0.0,
    };
    let carets: Vec<f32> = (0..count)
        .map(|c| columns[c] + CELL_PADDING + shift(c, widths[c]))
        .collect();
    // Each line's cells, set in rows no wider than their columns, and where the line ends.
    let set = parsed.into_iter().enumerate().map(|(index, cells)| {
        let (line, line_end) = places[index];
        // The row runs to its last cell, written in or not, and never past the closing bar.
        let written = cells.iter().flatten().filter_map(|g| g.at).max();
        let end = written
            .map(|at| at + 1)
            .into_iter()
            .chain(starts[index].last().copied())
            .max()
            .unwrap_or(line_end);
        let cells = cells.into_iter().enumerate().map(|(c, cell)| {
            let mut within = Vec::new();
            wrap(Row::new(line, end), cell, 0.0, widths[c], &mut within);
            within.into_iter().map(|row| row.glyphs).collect()
        });
        (cells.collect(), end)
    });
    let set: Vec<(Vec<Vec<Vec<Glyph>>>, usize)> = set.collect();
    // Where cells are all there is, nothing tells a line of the table from the rows a wrapped
    // cell takes, so once any cell wraps a rule goes between the lines.
    let wrapped = set
        .iter()
        .any(|(cells, _)| cells.iter().any(|cell| cell.len() > 1));
    for (index, (cells, end)) in set.into_iter().enumerate() {
        let line = places[index].0;
        let table = |part: usize| TableRow {
            columns: columns.clone(),
            carets: carets.clone(),
            header: index == 0,
            part,
        };
        if ruled && wrapped && index > 0 {
            let across = columns.last().copied().unwrap_or(0.0) as usize;
            let dashes = (0..across).flat_map(|x| {
                let crossing = columns[1..count]
                    .iter()
                    .any(|bar| (bar - x as f32).abs() < 0.5);
                let mut dash = decoration(if crossing { "┼" } else { "─" }, Tone::TableLine);
                dash[0].x = x as f32;
                dash[0].advance = 1.0;
                dash
            });
            rows.push(Row {
                glyphs: dashes.collect(),
                cells: starts[index].clone(),
                stop: false,
                table: Some(table(0)),
                ..Row::new(line, end)
            });
        }
        let parts = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
        for part in 0..parts {
            let mut glyphs = Vec::new();
            for (c, left) in columns.iter().take(count).enumerate() {
                if c > 0 {
                    let mut bar = decoration("│", Tone::TableLine);
                    bar[0].x = *left;
                    bar[0].advance = 1.0;
                    glyphs.extend(bar);
                }
                let Some(text) = cells.get(c).and_then(|cell| cell.get(part)) else {
                    continue;
                };
                // The space a row was broken at stands past its words, and isn't weighed.
                let spaces = text.iter().rev().take_while(|g| g.ch == ' ').count();
                let shown = &text[..text.len() - spaces];
                let used: f32 = shown.iter().map(|g| g.advance).sum();
                let at = left + CELL_PADDING + shift(c, (widths[c] - used).max(0.0));
                glyphs.extend(text.iter().map(|g| Glyph { x: at + g.x, ..*g }));
            }
            rows.push(Row {
                glyphs,
                cells: starts[index].clone(),
                lead: CELL_PADDING,
                table: Some(table(part)),
                ..Row::new(line, end)
            });
        }
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
    /// Suggestions that run across lines, which no one block sees whole: where each starts and
    /// ends, its kind, and where a replacement's arrow is.
    spanning: Vec<Spanning>,
}

/// A suggestion running across lines, as char places in the document.
struct Spanning {
    start: usize,
    end: usize,
    kind: cellix_doc::changes::Kind,
    arrow: usize,
}

impl Context<'_> {
    /// How a char inside a suggestion that runs across lines shows: as what it marks, or
    /// hidden when it's one of the suggestion's own markers. None outside them.
    fn across(&self, at: usize) -> Option<Option<Tone>> {
        use cellix_doc::changes::Kind;
        let span = self.spanning.iter().find(|s| at >= s.start && at < s.end)?;
        let marker = at < span.start + 3
            || at >= span.end - 3
            || (span.kind == Kind::Replace && (span.arrow..span.arrow + 2).contains(&at));
        if marker {
            return Some(None);
        }
        Some(Some(match span.kind {
            Kind::Insert => Tone::Inserted,
            Kind::Delete => Tone::Deleted,
            Kind::Replace if at < span.arrow => Tone::Deleted,
            Kind::Replace => Tone::Inserted,
        }))
    }
}

impl Context<'_> {
    /// A cell's pixels, or None on the cell tier, which has no pictures.
    fn cell(&self) -> Option<(u32, u32)> {
        self.measure.cell_pixels()
    }

    /// A footnote's number: the next one the first time it's met.
    fn footnote_number(&self, id: &str) -> usize {
        let mut seen = self.extras.footnotes.borrow_mut();
        match seen.iter().position(|known| known == id) {
            Some(index) => index + 1,
            None => {
                seen.push(id.to_string());
                seen.len()
            }
        }
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

    /// A diagram for this page, or why it can't be drawn. While its source is half typed,
    /// the last good picture stays, so the page doesn't jump.
    fn diagram(&self, kind: Kind, source: &str, width: f32, place: usize) -> Result<usize, String> {
        let cell = self.cell().ok_or_else(String::new)?;
        let wanted = (width * cell.0 as f32) as u32;
        let dark = self.extras.dark;
        let make = || {
            let svg = cellix_doc::diagram::svg(kind, source, dark)?;
            let raster = cellix_doc::diagram::rasterize(&svg, device_scale(cell), wanted)?;
            Ok(figure_of_raster(raster, (kind, source, dark), cell))
        };
        let figure: Result<Figure, String> = match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .diagrams
                .entry((kind, source.to_string(), wanted))
                .or_insert_with(make)
                .clone(),
            None => make(),
        };
        let Some(library) = self.extras.library else {
            return figure.map(|f| self.keep(f));
        };
        match figure {
            Ok(figure) => {
                library.borrow_mut().last_good.insert(place, figure.clone());
                Ok(self.keep(figure))
            }
            Err(why) => match library.borrow().last_good.get(&place).cloned() {
                Some(figure) => {
                    self.extras.broken.borrow_mut().push((place, why));
                    Ok(self.keep(figure))
                }
                None => Err(why),
            },
        }
    }

    /// Where each row of a display stands in its picture, kept between layouts.
    fn row_centres(&self, source: &str, count: usize) -> Option<Vec<f32>> {
        const EM: u32 = 24;
        if count < 2 {
            return None;
        }
        let expanded = self.extras.project.as_ref().map(|p| p.expand(source));
        let source = expanded.as_deref().unwrap_or(source);
        let make = || {
            let rows = cellix_doc::latex::display_rows(source);
            let rows: Vec<String> = rows.iter().map(ToString::to_string).collect();
            cellix_doc::math::row_centres(&rows, EM).filter(|c| c.len() == count)
        };
        match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .rows
                .entry(source.to_string())
                .or_insert_with(make)
                .clone(),
            None => make(),
        }
    }

    /// A picture a notebook cell drew.
    fn cell_picture(&self, bytes: &[u8], width: f32) -> Option<usize> {
        let cell = self.cell()?;
        let wanted = (width * cell.0 as f32) as u32;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        let key = hasher.finish();
        let make = || {
            let image = image::load_from_memory(bytes).ok()?.to_rgba8();
            let image = if image.width() > wanted {
                let height = image.height() * wanted / image.width();
                image::imageops::thumbnail(&image, wanted, height.max(1))
            } else {
                image
            };
            let (width, height) = image.dimensions();
            let raster = Raster {
                width,
                height,
                rgba: image.into_raw(),
            };
            Some(figure_of_raster(raster, key, cell))
        };
        let figure = match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .built
                .entry((key, wanted))
                .or_insert_with(make)
                .clone(),
            None => make(),
        };
        figure.map(|f| self.keep(f))
    }

    /// A piece of LaTeX for this page, once it's been built. Asking is what starts the build.
    fn built(&self, body: &str, width: f32) -> Built {
        let (Some(cell), Some(folder)) = (self.cell(), self.extras.built) else {
            return Built::Never;
        };
        let setup = self.extras.project.as_ref().map(|p| p.setup.as_slice());
        let tex = cellix_doc::latex::standalone(setup.unwrap_or_default(), body);
        let snippet = Snippet::new(tex);
        let key = snippet.key;
        let wanted = (width * cell.0 as f32) as u32;
        match crate::snippets::state(folder, key) {
            State::Missing => {
                self.extras.wanted.borrow_mut().push(snippet);
                Built::Waiting
            }
            State::Failed(reason) => Built::Failed(reason),
            State::Ready(path) => {
                let make = || {
                    let document = cellix_doc::pdf::Document::open(std::fs::read(&path).ok()?);
                    // A point is about a thirteenth of a row of text.
                    let scale = cell.1 as f32 / 13.5;
                    let page = document.ok()?.render_at(0, scale, wanted)?;
                    Some(figure_of_built(&page, key, cell))
                };
                let figure = match self.extras.library {
                    Some(library) => library
                        .borrow_mut()
                        .built
                        .entry((key, wanted))
                        .or_insert_with(make)
                        .clone(),
                    None => make(),
                };
                figure.map_or(Built::Never, |figure| Built::Shown(self.keep(figure)))
            }
        }
    }

    /// Loads an image for this page, and returns its index among the figures. LaTeX names
    /// files without their extension, and a figure is often a PDF.
    fn image(&self, source: &str, fit: Fit) -> Option<usize> {
        let cell = self.cell()?;
        let folder = self.extras.folder?;
        // A paper's pictures are found from its root's folder, and its `\graphicspath`, as
        // LaTeX finds them, even on the page of a file in a folder of its own.
        let folders: Vec<PathBuf> = match self.extras.project.as_deref() {
            Some(project) if !project.pictures.is_empty() => project.pictures.clone(),
            Some(project) => std::iter::once(folder.to_path_buf())
                .chain(project.graphicspath.iter().map(|path| folder.join(path)))
                .collect(),
            None => vec![folder.to_path_buf()],
        };
        let path = folders.iter().find_map(|folder| {
            let named = folder.join(source);
            ["", "png", "jpg", "jpeg", "pdf", "webp", "svg"]
                .iter()
                .map(|extension| {
                    if extension.is_empty() {
                        named.clone()
                    } else {
                        named.with_extension(extension)
                    }
                })
                .find(|path| path.is_file())
        })?;
        let make = || load_image(&path, cell, fit);
        let figure = match self.extras.library {
            Some(library) => library
                .borrow_mut()
                .images
                .entry((path.clone(), (fit.cells() as u32) << 1 | exact(fit)))
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

/// Tells a size asked for from a limit, in the image cache's key.
fn exact(fit: Fit) -> u32 {
    u32::from(matches!(fit, Fit::Exactly(_)))
}

/// How wide a picture stands, given the share of the page its source asks for.
fn fit_for(share: Option<u32>, width: f32) -> Fit {
    match share {
        Some(percent) => Fit::Exactly(width * percent.clamp(5, 100) as f32 / 100.0),
        None => Fit::AtMost(width.min(90.0)),
    }
}

/// Sets equation numbers at the right margin, spread down the picture's rows.
fn margin_numbers(
    (numbers, source): (&[Option<String>], &str),
    first: usize,
    height: f32,
    width: f32,
    context: &Context,
    rows: &mut [Row],
) {
    let count = numbers.len().max(1) as f32;
    let centres = context.row_centres(source, numbers.len());
    for (index, number) in numbers.iter().enumerate() {
        let Some(number) = number else {
            continue;
        };
        // In the middle of its line, in a picture that starts a little into its row.
        let share = centres
            .as_ref()
            .and_then(|c| c.get(index).copied())
            .unwrap_or((index as f32 + 0.5) / count);
        let middle = 0.2 + height * share;
        let row = first + middle.floor() as usize;
        let shift = ((middle.fract() - 0.5) * 64.0) as i8;
        let number = number.replace('$', "").replace("\\ast", "∗");
        let mut mark = decoration(&format!("({number})"), Tone::Margin(shift));
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

/// What a notebook's cell printed and drew, under it.
fn cell_outputs(
    outputs: &[Output],
    (line, at): (usize, usize),
    width: f32,
    context: &Context,
    rows: &mut Vec<Row>,
) {
    /// Lines of printed text shown before the rest is left out.
    const MOST: usize = 40;
    if !outputs.is_empty() {
        rows.push(spacer(line, at));
    }
    for output in outputs {
        match output {
            Output::Text(text) | Output::Error(text) => {
                let tone = if matches!(output, Output::Error(_)) {
                    Tone::Failed
                } else {
                    Tone::Dim
                };
                let lines: Vec<&str> = text.lines().collect();
                for shown in lines.iter().take(MOST) {
                    let mut row = note_row(shown, line, at, context);
                    for g in &mut row.glyphs {
                        g.tone = tone;
                        g.font.mono = true;
                    }
                    measure_glyphs(&mut row.glyphs, context.measure);
                    let mut x = CODE_PADDING;
                    for g in &mut row.glyphs {
                        g.x = x;
                        x += g.advance;
                    }
                    rows.push(row);
                }
                if lines.len() > MOST {
                    let more = format!("{} more lines", lines.len() - MOST);
                    rows.push(note_row(&more, line, at, context));
                }
            }
            Output::Image(bytes) => {
                if let Some(figure) = context.cell_picture(bytes, width.min(90.0)) {
                    figure_rows(figure, context, (line, at, at), width, rows);
                    if let Some(row) = rows.iter_mut().rfind(|r| r.figure.is_some()) {
                        row.stop = false;
                    }
                }
            }
        }
    }
}

/// A small line under something, saying what's wrong with it.
fn note_row(text: &str, line: usize, at: usize, context: &Context) -> Row {
    let mut note = decoration(text, Tone::Dim);
    for g in &mut note {
        g.font.size = 85;
    }
    measure_glyphs(&mut note, context.measure);
    let mut x = CODE_PADDING;
    for g in &mut note {
        g.x = x;
        x += g.advance;
    }
    Row {
        glyphs: note,
        ..spacer(line, at)
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
    // Only Markdown's marks are found here; a paper's go through its own commands.
    let spanning = if extras.dialect == Dialect::Latex {
        Vec::new()
    } else {
        cellix_doc::changes::find(text, cellix_doc::changes::Syntax::Critic)
            .into_iter()
            .filter(|change| {
                let chars = text
                    .chars()
                    .skip(change.start)
                    .take(change.end - change.start);
                chars.into_iter().any(|c| c == '\n')
            })
            .map(|change| Spanning {
                arrow: change.start + 3 + change.old.chars().count(),
                start: change.start,
                end: change.end,
                kind: change.kind,
            })
            .collect()
    };
    let context = Context {
        measure,
        registry,
        extras,
        spanning,
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
    if let Dialect::Data(separator) = extras.dialect {
        data::layout(&lines, separator, width, &context, &mut rows);
        return rows;
    }
    let mut i = 0;
    let mut callout = None;
    let plain: Vec<&str> = lines.iter().map(|(_, line)| *line).collect();
    let numbers = list_numbers(&plain);
    // The notebook cell the next fence belongs to.
    let mut cell = None;
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
        // The line that says which cell this is isn't part of the document.
        if let (Some(_), Some(mark)) = (extras.notebook, notebook::marker(line)) {
            cell = mark.cell.filter(|_| mark.code);
            i += 1;
            continue;
        }
        if let Some((fence, language)) = cellix_doc::fence::Fence::open(line) {
            let close = (i + 1..lines.len())
                .find(|n| fence.closes(lines[*n].1))
                .unwrap_or(lines.len());
            if cellix_doc::diagram::is_tikz(language) && context.cell().is_some() {
                let last = close.min(lines.len() - 1);
                let source: Vec<&str> = lines[i + 1..close].iter().map(|(_, l)| *l).collect();
                let body = cellix_doc::diagram::fenced_tikz(language, &source.join("\n"));
                let end = lines[last].0 + lines[last].1.chars().count();
                extras.spans.borrow_mut().push((offset, end + 1));
                let open = extras.open == Some((offset, end + 1));
                let inside = lines.get(i + 1).map_or(offset + 3, |(start, _)| *start);
                let built = context.built(&body, width);
                let shown = matches!(built, Built::Shown(_));
                if !shown || open {
                    code_rows(&lines[i..=last], i, "", &context, None, &mut rows);
                }
                match built {
                    Built::Shown(figure) => {
                        figure_rows(figure, &context, (i, inside, end), width, &mut rows);
                        if open && let Some(row) = rows.iter_mut().rfind(|r| r.figure.is_some()) {
                            row.stop = false;
                        }
                    }
                    Built::Waiting => {
                        rows.push(note_row("Drawing with LaTeX…", i, offset, &context));
                    }
                    Built::Failed(why) => rows.push(note_row(&why, i, offset, &context)),
                    Built::Never => {}
                }
                i = close + 1;
                continue;
            }
            let kind = cellix_doc::diagram::kind_of(language).filter(|_| context.cell().is_some());
            let mut drawing = None;
            if let Some(kind) = kind {
                let last = close.min(lines.len() - 1);
                let end = lines[last].0 + lines[last].1.chars().count();
                extras.spans.borrow_mut().push((offset, end + 1));
                let open = extras.open == Some((offset, end + 1));
                let source: Vec<&str> = lines[i + 1..close].iter().map(|(_, l)| *l).collect();
                let drawn = context.diagram(kind, &source.join("\n"), width, offset);
                let inside = lines.get(i + 1).map_or(offset + 3, |(start, _)| *start);
                if let (false, Ok(figure)) = (open, &drawn) {
                    figure_rows(*figure, &context, (i, inside, end), width, &mut rows);
                    i = close + 1;
                    continue;
                }
                drawing = Some((drawn, (i, inside, end)));
            }
            if !language.is_empty() {
                // A cell says when it ran, as Jupyter's `[3]` does.
                let count = extras.notebook.zip(cell).and_then(|(b, n)| b.count(n));
                let shown = count.map_or_else(
                    || language.to_string(),
                    |count| format!("{language}  [{count}]"),
                );
                let mut label = decoration(&shown, Tone::Dim);
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
            if let (Some(book), Some(number)) = (extras.notebook, cell.take()) {
                let last = close.min(lines.len() - 1);
                let end = lines[last].0 + lines[last].1.chars().count();
                let outputs = book.outputs(number);
                cell_outputs(&outputs, (close.min(last), end), width, &context, &mut rows);
                if book.running.contains(&number) {
                    rows.push(note_row("Running…", close.min(last), end, &context));
                }
            }
            // Being written, it stands under its source, and says what's wrong with it.
            match drawing {
                Some((Ok(figure), place)) => {
                    figure_rows(figure, &context, place, width, &mut rows);
                    if let Some(row) = rows.iter_mut().rfind(|r| r.figure.is_some()) {
                        row.stop = false;
                    }
                    let broken = extras.broken.borrow();
                    if let Some((_, why)) = broken.iter().find(|(place, _)| *place == offset) {
                        rows.push(note_row(why, i, offset, &context));
                    }
                }
                Some((Err(why), _)) if !why.is_empty() => {
                    rows.push(note_row(&why, i, offset, &context));
                }
                _ => {}
            }
            i = close + 1;
            continue;
        }
        if line.contains('|') && lines.get(i + 1).is_some_and(|n| is_table_rule(n.1)) {
            let end = (i + 2..lines.len())
                .find(|n| !lines[*n].1.contains('|'))
                .unwrap_or(lines.len());
            table(&lines[i..end], i, width, &context, &mut rows);
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
                margin_numbers((&numbers, &formula), at, shown, width, &context, &mut rows);
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
        if let Some(image) = image_line(line) {
            let (indent, alt, path) = (image.indent, image.alt, image.path);
            let end = offset + line.chars().count();
            extras.spans.borrow_mut().push((offset, end + 1));
            let open = extras.open == Some((offset, end + 1));
            let figure = context.image(path, fit_for(image.width, width));
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
                // The picture stays in view under the line that names it.
                if let Some(figure) = figure {
                    let place = (i, offset + indent + 1, end);
                    figure_rows(figure, &context, place, width, &mut rows);
                    if let Some(row) = rows.iter_mut().rfind(|r| r.figure.is_some()) {
                        row.stop = false;
                    }
                }
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
        if let Some((id, start)) = footnote_definition(line) {
            let number = context.footnote_number(&id);
            let mut glyphs = decoration(&number.to_string(), Tone::Footnote);
            for g in &mut glyphs {
                g.font.size = 72;
            }
            glyphs.extend(decoration(" ", Tone::TableLine));
            let small = Look {
                font: Font {
                    size: 88,
                    ..Font::default()
                },
                ..PLAIN
            };
            let decorated = glyphs.len();
            inline(
                &chars[start.min(chars.len())..],
                offset + start,
                small,
                &context,
                &mut glyphs,
            );
            measure_glyphs(&mut glyphs, measure);
            let lead: f32 = glyphs[..decorated].iter().map(|g| g.advance).sum();
            wrap(
                Row::new(i, offset + chars.len()),
                glyphs,
                lead,
                width,
                &mut rows,
            );
            i += 1;
            continue;
        }
        let found = prefix(line);
        let title = found
            .as_ref()
            .filter(|p| matches!(p.marker, Marker::Quote))
            .and_then(|p| Callout::named(&line[p.hidden.min(line.len())..]));
        if matches!(found.as_ref().map(|p| &p.marker), Some(Marker::Quote)) {
            callout = title.as_ref().map(|(kind, _)| *kind).or(callout);
        } else {
            callout = None;
        }
        let mut glyphs = Vec::new();
        let mut template = Row::new(i, offset + chars.len());
        template.callout = callout;
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
                    Marker::Ordered(written) => {
                        // Counted as Markdown counts it, whatever the file says.
                        let number = numbers[i].map_or(*written, |(_, n)| n);
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
        let hidden = if title.is_some() { chars.len() } else { hidden };
        let decorated = glyphs.len();
        if let Some((kind, named)) = &title {
            let shown = named.as_deref().unwrap_or(kind.title());
            glyphs.extend(decoration(shown, Tone::Callout(*kind)));
            for g in &mut glyphs {
                g.font.bold = true;
            }
        }
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
            let indent = found.as_ref().map_or(0, |p| p.indent);
            // Nesting moves the marker too, so it keeps its distance from its text.
            for g in glyphs.iter_mut().take(indent.min(decorated)) {
                g.advance = 1.0;
                g.tone = Tone::TableLine;
            }
            let pad = indent as f32;
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
            let index = table_part(rows, index, pos);
            return (index, rows[index].caret_x(pos));
        }
    }
    let last = rows.iter().rposition(|r| r.stop).unwrap_or(0);
    (last, rows.get(last).map_or(0.0, |r| r.caret_x(usize::MAX)))
}

/// The rows a table's source line stands on, given any one of them.
fn table_parts(rows: &[Row], index: usize) -> std::ops::Range<usize> {
    let line = rows[index].line;
    let same = |r: &Row| r.line == line && r.table.is_some();
    if !same(&rows[index]) {
        return index..index + 1;
    }
    let first = (0..index)
        .rev()
        .take_while(|i| same(&rows[*i]))
        .last()
        .unwrap_or(index);
    let after = rows[index..].iter().take_while(|r| same(r)).count();
    first..index + after
}

/// The places of the source a row shows from one of its cells.
fn cell_places(row: &Row, cell: usize) -> impl Iterator<Item = usize> + '_ {
    let start = row.cells[cell];
    let end = row.cells.get(cell + 1).copied().unwrap_or(usize::MAX);
    let places = row.glyphs.iter().filter_map(|g| g.at);
    places.filter(move |at| (start..end).contains(at))
}

/// Which row of a table's line holds `pos`, given the line's first: the first whose share of
/// the cell reaches it, or for the cell's end, the last with any of the cell on it.
fn table_part(rows: &[Row], first: usize, pos: usize) -> usize {
    let row = &rows[first];
    if row.table.is_none() {
        return first;
    }
    let Some(cell) = row.cell_holding(pos) else {
        return first;
    };
    let mut last = first;
    for index in table_parts(rows, first) {
        let mut places = cell_places(&rows[index], cell).peekable();
        if places.peek().is_none() {
            continue;
        }
        last = index;
        if places.any(|at| at >= pos) {
            return index;
        }
    }
    last
}

/// The source position nearest a column of a row. On a row of a table where the cell under
/// that column has run out of text, it's the end of that cell's text, which is above.
pub fn position_in(rows: &[Row], index: usize, column: f32) -> usize {
    let row = &rows[index];
    let Some(cell) = blank_cell(row, column) else {
        return row.position_at(column);
    };
    let above = table_parts(rows, index).start..index;
    let ends = above.filter_map(|i| cell_places(&rows[i], cell).max());
    ends.max().map_or(row.cells[cell], |at| at + 1)
}

/// The cell under a column, when this row is a later row of its table line and that cell
/// shows nothing on it.
fn blank_cell(row: &Row, column: f32) -> Option<usize> {
    let table = row.table.as_ref().filter(|t| t.part > 0)?;
    let last = table.columns.len().checked_sub(2)?;
    let cell = table.columns.iter().rposition(|x| *x <= column)?.min(last);
    let cell = cell.min(row.cells.len().checked_sub(1)?);
    cell_places(row, cell).next().is_none().then_some(cell)
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
    // A row's end and a glyph's own place are places already. Going through `x` for them
    // can land elsewhere, as at the end of a reference shown as its number.
    if pos == row.end || row.glyphs.iter().any(|g| g.at == Some(pos)) {
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
    // A row of a table where this column's cell has run out of text isn't a place to stop.
    let stop = |r: &Row| r.stop && blank_cell(r, goal).is_none();
    for _ in 0..delta.unsigned_abs() {
        let next = if delta < 0 {
            rows[..row].iter().rposition(stop)
        } else {
            rows.iter()
                .skip(row + 1)
                .position(stop)
                .map(|i| row + 1 + i)
        };
        match next {
            Some(found) => row = found,
            None => break,
        }
    }
    if row < rows.len() {
        position_in(rows, row, goal)
    } else {
        pos
    }
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

/// The table cell before the one at `pos`, for Shift+Tab.
pub fn previous_cell(rows: &[Row], pos: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    let here = rows.get(row)?;
    if here.cells.is_empty() {
        return None;
    }
    let first = (0..=row)
        .rev()
        .take_while(|n| !rows[*n].cells.is_empty() || !rows[*n].stop)
        .last()?;
    let mut before = rows[first..=row]
        .iter()
        .flat_map(|r| r.cells.iter().copied());
    let current = here.cells[here.cell_holding(pos)?];
    before.rfind(|at| *at < current)
}

/// The cell under the one at `pos`, for Enter.
pub fn cell_below(rows: &[Row], pos: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    let here = rows.get(row)?;
    let column = here.cell_holding(pos)?;
    let row = table_parts(rows, row).end - 1;
    let below = rows[row + 1..].iter().find(|r| !r.cells.is_empty());
    let between = rows[row + 1..]
        .iter()
        .take_while(|r| r.cells.is_empty())
        .all(|r| !r.stop);
    let below = below.filter(|_| between)?;
    below.cells.get(column.min(below.cells.len() - 1)).copied()
}

/// The start of a column of the row at `pos`, for a row just added.
pub fn cell_in_row(rows: &[Row], pos: usize, column: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    let cells = &rows.get(row)?.cells;
    cells.get(column.min(cells.len().checked_sub(1)?)).copied()
}

/// Which column of its row `pos` is in.
pub fn cell_column(rows: &[Row], pos: usize) -> Option<usize> {
    let (row, _) = locate(rows, pos);
    rows.get(row)?.cell_holding(pos)
}

/// What a cell holds, as a range to select, so typing replaces it as it does in a sheet.
pub fn cell_range(rows: &[Row], start: usize) -> (usize, usize) {
    let (row, _) = locate(rows, start);
    let Some(here) = rows.get(row) else {
        return (start, start);
    };
    let Some(index) = here.cell_holding(start) else {
        return (start, start);
    };
    let parts = table_parts(rows, row);
    let last = parts
        .filter_map(|i| cell_places(&rows[i], index).max())
        .max();
    (start, last.map_or(start, |at| at + 1))
}

/// The numbers a numbered list shows, as Markdown counts them: on from the first item's,
/// whatever the others say. For each line that's an item, the line its list starts on and
/// its number. A line set in no further than an item ends its list, unless it's the next
/// item; blank lines and what's set in further, as an item's own text and lists, don't.
pub fn list_numbers(lines: &[&str]) -> Vec<Option<(usize, u64)>> {
    // The lists open here, outermost first: each one's indent, first line and next number.
    let mut open: Vec<(usize, usize, u64)> = Vec::new();
    let mut numbers = vec![None; lines.len()];
    let mut fence: Option<cellix_doc::fence::Fence> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if open.closes(line) {
                fence = None;
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if let Some(found) = prefix(line)
            && let Marker::Ordered(number) = found.marker
        {
            while open.last().is_some_and(|(at, ..)| *at > indent) {
                open.pop();
            }
            match open.last_mut() {
                Some((at, first, next)) if *at == indent => {
                    numbers[index] = Some((*first, *next));
                    *next += 1;
                }
                _ => {
                    open.push((indent, index, number + 1));
                    numbers[index] = Some((index, number));
                }
            }
            continue;
        }
        while open.last().is_some_and(|(at, ..)| *at >= indent) {
            open.pop();
        }
        fence = cellix_doc::fence::Fence::open(line).map(|(open, _)| open);
    }
    numbers
}

/// What to change so the numbered lists with an item near `line` count on from their
/// first item: each as (line, chars its number takes, the number it should be). A list
/// whose items all say the same number is left as it is, since that's written on purpose
/// and shows counted anyway.
pub fn renumbering(lines: &[&str], line: usize) -> Vec<(usize, usize, u64)> {
    let numbers = list_numbers(lines);
    let near = line.saturating_sub(2)..=line + 2;
    let mut lists: Vec<usize> = numbers
        .iter()
        .enumerate()
        .filter(|(index, _)| near.contains(index))
        .filter_map(|(_, number)| number.map(|(first, _)| first))
        .collect();
    lists.sort_unstable();
    lists.dedup();
    let mut changes = Vec::new();
    for list in lists {
        let items: Vec<(usize, u64)> = numbers
            .iter()
            .enumerate()
            .filter_map(|(index, n)| {
                n.filter(|(first, _)| *first == list)
                    .map(|(_, n)| (index, n))
            })
            .collect();
        let written = |index: usize| {
            let trimmed = lines[index].trim_start();
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            (digits, trimmed[..digits].parse::<u64>().unwrap_or(0))
        };
        let same = items
            .iter()
            .all(|(index, _)| written(*index).1 == written(list).1);
        if same && items.len() > 1 {
            continue;
        }
        for (index, shown) in items {
            let (digits, number) = written(index);
            if number != shown {
                changes.push((index, digits, shown));
            }
        }
    }
    changes
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

    #[test]
    fn a_callout_shows_its_own_title() {
        let rows = layout(
            "> [!tip]- Keep it short\n> Say less.\n",
            60,
            &CellMeasure,
            &Registry::default(),
        );
        let all: Vec<String> = rows.iter().map(text).collect();
        assert!(all[0].contains("Keep it short"), "{all:?}");
        assert!(!all.concat().contains("[!"), "{all:?}");
        assert_eq!(rows[0].callout, Some(Callout::Tip));
        assert!(all[1].contains("Say less."), "{all:?}");
    }
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
    fn a_suggestion_across_paragraphs_shows_without_its_markers() {
        let source = "Keep.\n\n{++New para one.\n\nNew para two.++}\n\n{--Gone one.\n\nGone two.--}\n\nEnd.\n";
        let rows = page(source, 40);
        let all: String = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        for marker in ["{++", "++}", "{--", "--}"] {
            assert!(!all.contains(marker), "{marker} shows: {all}");
        }
        let tone_of = |word: &str| {
            let row = rows.iter().find(|r| text(r).contains(word)).expect(word);
            row.glyphs
                .iter()
                .find(|g| g.ch.is_alphabetic())
                .map(|g| g.tone)
        };
        assert_eq!(tone_of("New para two."), Some(Tone::Inserted));
        assert_eq!(tone_of("New para one."), Some(Tone::Inserted));
        assert_eq!(tone_of("Gone two."), Some(Tone::Deleted));
        assert_ne!(tone_of("End."), Some(Tone::Inserted));
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
    fn output_holding_a_shorter_fence_stays_inside_its_block() {
        // As a transcript writes a step's output that has a fence of its own: four outside.
        let source = "````\n```\n# Approve this\n[click](https://x.test)\n```\n````\n## Turn 2";
        let rows = page(source, 60);
        let heading = rows
            .iter()
            .position(|r| text(r) == "Turn 2")
            .expect("the heading after the block");
        for word in ["# Approve this", "[click](https://x.test)", "```"] {
            let row = rows[..heading].iter().find(|r| text(r) == word);
            assert!(
                row.is_some_and(|r| r.shaded),
                "{word} as code, inside the block"
            );
        }
        assert!(!rows[heading].shaded);
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

    const WIDE_TABLE: &str = "| Name | What it does |\n|---|---|\n| queue | Refuses work once it is full so a fast producer cannot outrun it |\n| last | row |";

    #[test]
    fn a_table_wider_than_the_page_wraps_its_cells_to_fit() {
        let rows = page(WIDE_TABLE, 40);
        let shown: Vec<String> = rows.iter().map(text).collect();
        let table = |r: &Row| r.table.clone().unwrap();
        let edge = table(&rows[0]).columns.last().copied().unwrap();
        assert!(edge <= 40.0, "the table is {edge} wide on a page of 40");
        // The long line of the table takes several rows, its words in their own column.
        let long: Vec<&Row> = rows.iter().filter(|r| r.line == 2 && r.stop).collect();
        assert!(long.len() > 1, "{shown:?}");
        let second = table(long[0]).columns[1];
        let in_second = |r: &&Row| -> String {
            let said = r.glyphs.iter().filter(|g| g.at.is_some() && g.x >= second);
            said.map(|g| g.ch).collect()
        };
        let words: String = long.iter().map(in_second).collect();
        assert_eq!(
            words,
            "Refuses work once it is full so a fast producer cannot outrun it"
        );
        let glyphs = rows.iter().flat_map(|r| &r.glyphs);
        assert!(glyphs.clone().all(|g| g.x < edge), "{shown:?}");
        assert!(text(long[0]).contains("queue") && !text(long[1]).contains("queue"));
        // A word that fills its row exactly keeps its space: the next row starts with a word.
        let exact = page("abcdefg hij", 7);
        assert_eq!(text(&exact[1]), "hij");
        assert_eq!(table(long[1]).part, 1);
        // On cells, a rule parts the lines of a table whose cells wrap. With pictures its box
        // does that.
        let rules = |rows: &[Row]| {
            rows.iter()
                .filter(|r| !r.stop && text(r).starts_with('─'))
                .count()
        };
        assert_eq!(rules(&rows), 2, "{shown:?}");
        let painted = layout(
            WIDE_TABLE,
            40,
            &FontMeasure::new(10, 20),
            &Registry::default(),
        );
        assert_eq!(rules(&painted), 0);
        // One that fits is as it was: a row for each line.
        assert_eq!(page("| a | b |\n|---|---|\n| c | d |", 40).len(), 2);
        // A word longer than the page, as a link can be, is broken to keep the table in it.
        let link = "https://example.com/a/very/long/path/that/never/breaks";
        let linked = page(&format!("| a | b |\n|---|---|\n| {link} | x |"), 30);
        let edge = table(&linked[0]).columns.last().copied().unwrap();
        assert!(edge <= 30.0, "{edge}");
        let first = table(&linked[0]).columns[1];
        let said = linked.iter().filter(|r| r.line == 2 && r.stop);
        let said = said
            .flat_map(|r| &r.glyphs)
            .filter(|g| g.x < first && g.at.is_some());
        assert_eq!(said.map(|g| g.ch).collect::<String>(), link);
        // And it's broken after a slash, as a person would break it.
        let broken = linked
            .iter()
            .filter(|r| r.line == 2 && r.stop && !r.glyphs.is_empty());
        let ends: Vec<char> = broken
            .filter_map(|r| r.glyphs.iter().rfind(|g| g.x < first && g.at.is_some()))
            .map(|g| g.ch)
            .collect();
        assert!(
            ends.len() > 1 && ends[..ends.len() - 1].iter().all(|c| *c == '/'),
            "{ends:?}"
        );
        // Numbers too wide for a narrow column's share stay whole: the table is wider instead.
        let numbers = page(
            "| a | b | c | d |\n|---|---|---|---|\n| 1000.5 | 2000.5 | 3000.5 | 4000.5 |",
            16,
        );
        assert_eq!(
            numbers.len(),
            2,
            "{:?}",
            numbers.iter().map(text).collect::<Vec<_>>()
        );
        // Only more columns than the page has room for leave a table wider than it.
        let many = page(
            "| a | b | c | d | e | f |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |",
            12,
        );
        assert!(table(&many[0]).columns.last().copied().unwrap() > 12.0);
    }

    #[test]
    fn the_caret_finds_its_way_through_a_wrapped_cell() {
        let rows = page(WIDE_TABLE, 40);
        let chars: Vec<char> = WIDE_TABLE.chars().collect();
        let at = |words: &str| WIDE_TABLE.find(words).unwrap();
        let long = (0..rows.len()).filter(|i| rows[*i].line == 2 && rows[*i].stop);
        let long: Vec<usize> = long.collect();
        // A word further on in the cell is on a later row of it, under the cell's first.
        let (first, x) = locate(&rows, at("Refuses"));
        assert_eq!(first, long[0]);
        let (later, _) = locate(&rows, at("outrun"));
        assert!(later > first && rows[later].line == 2);
        // Down from its first word stays in the cell, a row on.
        let down = vertical(&rows, at("Refuses"), 1, x);
        assert_eq!(locate(&rows, down).0, long[1]);
        assert!(down > at("Refuses") && down < at("| last"));
        // Down from the short cell beside it goes past the rows it isn't on, to the next line.
        let (_, short) = locate(&rows, at("queue"));
        let below = vertical(&rows, at("queue"), 1, short);
        assert_eq!(chars[below], 'l');
        assert_eq!(vertical(&rows, below, -1, short), at("queue"));
        // A click under the short cell's text is at that text's end.
        assert_eq!(position_in(&rows, long[1], short), at("queue") + 5);
        // The cell is all of its rows, to select and to leave by Enter.
        let end = at("outrun it") + "outrun it".len();
        assert_eq!(cell_range(&rows, at("Refuses")), (at("Refuses"), end));
        assert_eq!(cell_below(&rows, at("Refuses")), Some(at("row")));
        // Its end is on its last row, after its last word.
        let (last, end_x) = locate(&rows, end);
        assert_eq!(last, *long.last().unwrap());
        assert!(end_x > x);
        // And every place in the table is still reached by stepping through it.
        let mut pos = at("Name");
        let mut seen = vec![pos];
        while pos < at("row") {
            let next = step(&rows, pos, true);
            assert!(next > pos, "stuck at {pos}");
            pos = next;
            seen.push(pos);
        }
        assert!(seen.contains(&at("outrun")) && seen.contains(&at("last")));
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
        assert!(matches!(enter_intent("1. a"), Intent::Continue(s) if s == "2. "));
        assert!(matches!(enter_intent("  2. a"), Intent::Continue(s) if s == "  3. "));
        assert!(matches!(enter_intent("- [x] a"), Intent::Continue(s) if s == "- [ ] "));
        assert!(matches!(enter_intent("> a"), Intent::Continue(s) if s == "> "));
        assert!(matches!(enter_intent("- "), Intent::EndList(2)));
        assert!(matches!(enter_intent("# a"), Intent::Plain));
        assert!(matches!(enter_intent("plain"), Intent::Plain));
    }

    #[test]
    fn a_numbered_list_counts_on_from_its_first_item() {
        let lines = [
            "1. one",
            "3. three",
            "   text of three",
            "   - inside",
            "",
            "4. four",
            "Then a paragraph.",
            "7. new list",
            "```",
            "1. in code",
            "```",
        ];
        let shown: Vec<Option<u64>> = list_numbers(&lines)
            .iter()
            .map(|n| n.map(|(_, n)| n))
            .collect();
        assert_eq!(
            shown,
            [
                Some(1),
                Some(2),
                None,
                None,
                None,
                Some(3),
                None,
                Some(7),
                None,
                None,
                None
            ]
        );
        // Taking item 2 out leaves the next to count down to it; a list of all ones is left.
        assert_eq!(
            renumbering(&["1. a", "3. c", "4. d"], 1),
            [(1, 1, 2), (2, 1, 3)]
        );
        assert!(renumbering(&["1. a", "1. b", "1. c"], 1).is_empty());
        assert!(renumbering(&["1. a", "2. b", "", "x", "", "9. far", "1. away"], 0).is_empty());
        // Nested lists count on their own.
        let nested = ["1. a", "   1. x", "   3. y", "2. b"];
        assert_eq!(renumbering(&nested, 2), [(2, 1, 2)]);
    }
}
