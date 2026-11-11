use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cellix_core::{Buffer, Motion, Selection, Selections};
use cellix_doc::changes::{self, Change, Kind, Syntax};
use cellix_doc::latex::Project;
use cellix_doc::notebook::{Marker, Notebook, marker};
use cellix_doc::prose::{Issue, Source};
use cellix_doc::table::Edit as TableEdit;
use cellix_input::{Command, Direction, Format};
use cellix_lang::{Registry, Span, Token, Worker};
use cellix_render::{
    Bounds, Font, Grid, INTERFACE_SIZE, Icon, Measure, Paint, Rect, Rgb, Strip, units,
};
use cellix_review::LineChange;
use unicode_width::UnicodeWidthChar;

use crate::brackets::{self, Bracket};
use crate::editorconfig::{Indentation, Rules};
use crate::fold;
use crate::panels::Symbol;
use crate::rendered::{
    self, Callout, Dialect, Extras, Figure, Glyph, Intent, Library, Row, Target, Tone,
};
use crate::rows::{CodeRow, Rows};
use crate::snippets::Snippet;
use crate::theme::Theme;

/// The most header lines sticky scroll keeps at the top, as VS Code's default.
pub const STICKY_LINES: usize = 5;
const GUTTER_MIN_DIGITS: usize = 4;
const NARROW_PANE: u16 = 60;
/// Left margin of a rendered page, which also carries comment marks.
/// Heading levels the toolbar steps through before going back to plain text.
const MAX_HEADING_CYCLE: usize = 3;
const MINIMAP_WIDTH: u16 = 13;
/// Lines of code a minimap fits in one row of the grid.
const MINIMAP_LINES_PER_ROW: usize = 6;
/// Columns of code a minimap shows, past which lines are cut off.
const MINIMAP_COLUMNS: usize = 96;
/// Narrower panes spend every column on the code, as beside an agent.
const MIN_MINIMAP_PANE: u16 = 90;
/// Files up to this size highlight as they're drawn. Past it, a background thread does it,
/// because a full reparse on each key starts to show.
const MAX_INLINE_HIGHLIGHT: usize = 32 * 1024;
/// How far a tall heading sits below its row: closer to its own text than to what's above.
const TALL_DROP: f32 = 0.9;
/// Columns left of a page's text: a comment's ◆ and a change's bar each have a cell of
/// their own there at every width, clear of each other and of the text.
const PAGE_MARGIN: u16 = 4;
/// Columns right of it, which a narrow pane gives back to the text.
const NARROW_RIGHT_MARGIN: u16 = 2;
/// How wide a page can be set.
const MAX_PAGE_WIDTH: u16 = 512;

/// Per-line marks for the gutter, as 1-based lines.
#[derive(Default)]
pub struct Marks {
    pub comments: Vec<usize>,
    pub errors: Vec<usize>,
}

/// A page's rows, its figures, and where its formulas and images are.
type LaidOut = (
    Rc<Vec<Row>>,
    Vec<Figure>,
    Vec<(usize, usize)>,
    Option<usize>,
);

/// A laid-out page, and what it was made for.
struct PageCache {
    revision: u64,
    width: usize,
    /// The formula showing as source.
    open: Option<(usize, usize)>,
    rows: Rc<Vec<Row>>,
    figures: Rc<Vec<Figure>>,
    /// Every formula and image, which the caret opens by moving into.
    spans: Rc<Vec<(usize, usize)>>,
    /// The picture of a formula being edited in a line.
    preview: Option<usize>,
}

/// One open document and where the view is within it.
/// A colour pulled towards the background, as text is at a minimap's size.
fn dimmed(color: Rgb, bg: Rgb) -> Rgb {
    let mix = |a: u8, b: u8| ((a as u16 * 3 + b as u16 * 2) / 5) as u8;
    Rgb(mix(color.0, bg.0), mix(color.1, bg.1), mix(color.2, bg.2))
}

/// Bracket pairs, with the revision they were found at and whether strings were known.
type Pairs = ((u64, bool), Rc<Vec<Bracket>>);

/// Where code folds, with the revision and the server's answer it was worked out for.
type Regions = ((u64, usize), Rc<Vec<fold::Range>>);

/// What a code view's rows were laid out for: the revision, the wrap width, and the folds.
type RowsKey = (u64, Option<usize>, u64);

/// Bars per row, with the revision, height and width they fit.
type MinimapRows = ((u64, u16, u16), Vec<Vec<Strip>>);

pub struct Editor {
    pub buffer: Buffer,
    pub selections: Selections,
    /// Char ranges to mark, while find is open.
    pub found: Vec<(usize, usize)>,
    /// The last commit's copy of this file, to mark changed lines against.
    pub committed: Option<String>,
    /// Changed lines by number, with the revision they were worked out at.
    changes: RefCell<(u64, HashMap<usize, LineChange>)>,
    /// Bars for each row of the minimap, with what they were built for.
    minimap: RefCell<Option<MinimapRows>>,
    pub top: usize,
    pub left: usize,
    spans: RefCell<Option<(u64, Vec<Span>)>>,
    /// How many rows the pane showed last, which is what bounds scrolling.
    view_rows: Cell<u16>,
    /// Started for the first large file, and only then.
    worker: OnceCell<Worker>,
    /// The revision the worker was last asked about.
    asked: Cell<u64>,
    /// What the cached spans were read from, to carry them across an edit.
    spans_bytes: Cell<usize>,
    /// Show Markdown as an editable page rather than its source, as Xcode does.
    pub rendered: bool,
    /// What its tab says, when not its file's name: a transcript's page is named for its
    /// agent.
    pub label: Option<String>,
    /// The file changed on disk under unsaved edits, so saving would erase that change.
    pub conflict: bool,
    /// Whether the minimap may show, given the room.
    pub minimap_on: bool,
    /// The language server's inlay hints, as places in the text in order and what each
    /// says. They're drawn before the char at their place and are never part of the text.
    hints: Vec<(usize, String)>,
    /// The text's length and revision the hints were last moved to, so an edit carries them
    /// along until the server's new ones come.
    hints_for: (usize, u64),
    /// The revision and lines last asked about, so each is asked once.
    pub hints_asked: Option<(u64, usize, usize)>,
    /// The laid-out page, for one revision and width.
    page: RefCell<Option<PageCache>>,
    /// Formulas and images already drawn, so typing doesn't redo them.
    library: RefCell<Library>,
    /// Whether the sheet is dark, which diagrams are drawn for.
    dark: Cell<bool>,
    /// How wide the text was drawn, for what's drawn beside it.
    view_width: Cell<u16>,
    /// Whether typing here is kept as suggested changes.
    pub tracking: bool,
    /// While an agent's edits stand as suggestions nobody has touched: the text from before
    /// its first edit, and the buffer as Cellix last marked it. A later edit from the same
    /// turn is marked against that text again, so the edits pile up into one review.
    pub agent_review: Option<(String, String)>,
    /// The word count, how many files it took in, and what it was counted at: this file's
    /// revision, and a stamp of the other open files' unsaved edits.
    words: Cell<Option<Counted>>,
    /// The paper's other files as last read.
    paper_files: RefCell<cellix_doc::paper::Files>,
    /// The paper a LaTeX file belongs to, looked for again only once the file is saved.
    paper_root: RefCell<Option<PathBuf>>,
    /// The notebook this page was read from, which saving writes back into.
    pub notebook: Option<Notebook>,
    /// Spelling and grammar, for the revision they were checked at.
    pub issues: (u64, Vec<Issue>),
    /// The revision last sent to be checked.
    pub checked: u64,
    /// Where built pieces of LaTeX are kept.
    pub built: Option<PathBuf>,
    /// Pieces of LaTeX this page is waiting for, for the app to have built.
    wanted: RefCell<Vec<Snippet>>,
    /// Text comments are about, as char ranges, tinted on a page.
    pub quoted: RefCell<Vec<(usize, usize)>>,
    /// The paper this file belongs to, and the revision it was read at.
    project: RefCell<Option<(u64, Rc<Project>)>>,
    show_preamble: bool,
    measure: Rc<dyn Measure>,
    registry: Rc<Registry>,
    /// The revision last sent to a language server.
    pub synced: u64,
    /// Whether code's brackets are coloured by depth, with the caret's pair marked.
    pub bracket_pairs: bool,
    /// Bracket pairs, for the revision they were found at and whether strings were known.
    brackets: RefCell<Option<Pairs>>,
    /// Whether long lines of code wrap at the pane's width: Alt+Z.
    pub wrap: bool,
    /// The code's rows, for what they were laid out for.
    rows: RefCell<Option<(RowsKey, Rc<Rows>)>>,
    /// Folded regions by their header line and its text, so a fold follows its line through
    /// edits, with the revision and line count they were last followed at.
    folded: RefCell<Folded>,
    /// Where the language server says the code folds, as (first line, last line), with how
    /// many answers it has given, so each new one is noticed.
    server_folds: Option<(usize, Vec<(usize, usize)>)>,
    /// Where the code can fold, for the revision and server answer it was worked out for.
    fold_ranges: RefCell<Option<Regions>>,
    /// How many header lines of the scopes scrolled into may stay at the top: 0 for none.
    pub sticky_lines: usize,
    /// What a level of indentation is here, from `.editorconfig` or the default.
    pub indentation: Indentation,
    /// What saving does to the lines, from `.editorconfig`.
    pub rules: Rules,
}

/// Folded header lines with their text, and the revision and line count they're right for.
#[derive(Default)]
struct Folded {
    revision: u64,
    lines: usize,
    headers: Vec<(usize, String)>,
}

/// A word count, how many files it took in, and what it was counted at: the file's revision and a
/// stamp of the other open files' unsaved edits.
type Counted = ((u64, u64), usize, usize);

fn display_width(c: char, column: usize, tab: usize) -> usize {
    if c == '\t' {
        tab - column % tab
    } else {
        c.width().unwrap_or(0)
    }
}

/// The colour a callout's bar and title take.
fn callout_colour(kind: Callout, theme: &Theme) -> Rgb {
    match kind {
        Callout::Note => theme.accent,
        Callout::Tip => theme.inserted(),
        Callout::Important => Rgb(0x88, 0x57, 0xe0),
        Callout::Warning => Rgb(0xc0, 0x86, 0x10),
        Callout::Caution => theme.deleted(),
    }
}

/// How far below a paper's figure the line naming its image can be.
const FIGURE_LINES: usize = 6;

impl Editor {
    pub fn new(buffer: Buffer, measure: Rc<dyn Measure>, registry: Rc<Registry>) -> Self {
        // A file of definitions is code: it has no page to show.
        let definitions = |text: &str| {
            let lines = text.lines().filter(|l| !l.trim().is_empty());
            let marks = [
                "\\newcommand",
                "\\renewcommand",
                "\\def",
                "\\usepackage",
                "\\Declare",
            ];
            let (total, defining) = lines.fold((0, 0), |(total, defining), line| {
                let is = marks.iter().any(|m| line.trim_start().starts_with(m));
                (total + 1, defining + usize::from(is))
            });
            total > 0 && defining * 2 > total
        };
        let is_page =
            buffer.path().is_some_and(|p| Dialect::of(p).is_some()) && !definitions(&buffer.text());
        let rules = buffer.path().map(Rules::for_file).unwrap_or_default();
        let indentation = rules.indentation(Indentation::default());
        Self {
            buffer,
            selections: Selections::default(),
            found: Vec::new(),
            committed: None,
            changes: RefCell::new((u64::MAX, HashMap::new())),
            minimap: RefCell::new(None),
            top: 0,
            left: 0,
            spans: RefCell::new(None),
            view_rows: Cell::new(0),
            worker: OnceCell::new(),
            asked: Cell::new(u64::MAX),
            spans_bytes: Cell::new(0),
            rendered: is_page,
            label: None,
            conflict: false,
            minimap_on: true,
            hints: Vec::new(),
            hints_for: (0, 0),
            hints_asked: None,
            page: RefCell::new(None),
            library: RefCell::default(),
            dark: Cell::new(false),
            view_width: Cell::new(0),
            tracking: false,
            agent_review: None,
            words: Cell::new(None),
            paper_files: RefCell::default(),
            paper_root: RefCell::new(None),
            notebook: None,
            issues: (0, Vec::new()),
            checked: 0,
            built: None,
            wanted: RefCell::default(),
            quoted: RefCell::default(),
            project: RefCell::new(None),
            show_preamble: false,
            measure,
            registry,
            synced: 0,
            bracket_pairs: true,
            brackets: RefCell::new(None),
            wrap: false,
            rows: RefCell::new(None),
            folded: RefCell::default(),
            server_folds: None,
            fold_ranges: RefCell::new(None),
            sticky_lines: STICKY_LINES,
            indentation,
            rules,
        }
    }

    pub fn title(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        self.buffer
            .path()
            .and_then(Path::file_name)
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// Whether a pane has room for a minimap, which only pixels can draw.
    fn shows_minimap(&self, area: Rect) -> bool {
        // A document that fits has nothing for a minimap to show.
        let fits = self.rendered && {
            let cache = self.page.borrow();
            cache
                .as_ref()
                .is_some_and(|p| p.rows.len() <= area.h as usize)
        };
        self.minimap_on && !self.measure.in_cells() && area.w >= MIN_MINIMAP_PANE && !fits
    }

    /// Where text goes in `area`: all of it, less the minimap or the scrollbar.
    fn text_area(&self, area: Rect) -> Rect {
        let side = if self.shows_minimap(area) {
            MINIMAP_WIDTH
        } else {
            1
        };
        Rect::new(area.x, area.y, area.w.saturating_sub(side), area.h)
    }

    /// Forgets the changed lines, as when the file's committed copy changes.
    pub fn refresh_changes(&self) {
        self.changes.borrow_mut().0 = u64::MAX;
    }

    fn line_changes(&self) -> std::cell::Ref<'_, (u64, HashMap<usize, LineChange>)> {
        let revision = self.buffer.revision();
        if self.changes.borrow().0 != revision {
            let found = self.committed.as_ref().map_or_else(HashMap::new, |old| {
                cellix_review::line_changes(old, &self.buffer.text())
                    .into_iter()
                    .collect()
            });
            *self.changes.borrow_mut() = (revision, found);
        }
        self.changes.borrow()
    }

    /// The bar beside a changed line, in the margin of `x`.
    fn draw_change(&self, grid: &mut Grid, x: f32, y: u16, change: LineChange, theme: &Theme) {
        let color = match change {
            LineChange::Added => theme.git_added,
            LineChange::Modified => theme.git_modified,
            LineChange::DeletedAbove => theme.error,
        };
        let bounds = match change {
            LineChange::DeletedAbove => Bounds::cells(x, y as f32 - 0.06, 0.55, 0.12),
            _ => Bounds::cells(x, y as f32, 0.28, 1.0),
        };
        grid.panel(bounds, 0, Some(color), None);
        if self.measure.in_cells() {
            let style = cellix_render::Style {
                fg: color,
                ..theme.gutter
            };
            grid.put_str(x as u16, y, "▎", style, x as u16 + 1);
        }
    }

    /// The words of a line of code, as (column, length, colour).
    fn code_words(&self, line: usize, spans: &[Span], theme: &Theme) -> Vec<(usize, usize, Rgb)> {
        let start = self.buffer.rope().line_to_byte(line);
        let text = self.line_text(line);
        let mut words = Vec::new();
        let (mut column, mut byte) = (0, 0);
        for word in text.split_inclusive(char::is_whitespace) {
            let size = word.trim_end().chars().count();
            if size > 0 {
                let at = start + byte;
                let found = spans.partition_point(|s| s.end <= at);
                let span = spans.get(found).filter(|s| s.start <= at);
                let color = span.map_or(theme.text.fg, |s| theme.token(s.token));
                words.push((column, size, color));
            }
            column += word.chars().count();
            byte += word.len();
        }
        words
    }

    /// The words of a page row, in the same shape.
    fn page_words(row: &Row, theme: &Theme) -> Vec<(usize, usize, Rgb)> {
        let color = |tone| match tone {
            Tone::Heading => theme.heading,
            Tone::Code | Tone::Block(_) => theme.string,
            Tone::Link | Tone::Check(_) => theme.accent,
            _ => theme.text.fg,
        };
        let mut words = Vec::new();
        let mut run: Option<(usize, usize, Rgb)> = None;
        for g in &row.glyphs {
            if g.ch.is_whitespace() {
                words.extend(run.take());
            } else {
                let column = g.x as usize;
                let word = run.get_or_insert((column, 0, color(g.tone)));
                word.1 = (column + 1).saturating_sub(word.0);
            }
        }
        words.extend(run);
        words
    }

    /// Bars for each row of a minimap `rows` tall, in the colours of the code. The whole
    /// file always fits, so the map's fill shows how long the file is.
    fn minimap_rows(&self, rows: u16, text: Rect, theme: &Theme) -> Vec<Vec<Strip>> {
        let page = self.rendered.then(|| self.page(text));
        let lines = page
            .as_ref()
            .map_or_else(|| self.buffer.rope().len_lines(), |p| p.len());
        let capacity = rows as usize * MINIMAP_LINES_PER_ROW;
        let shown = lines.min(capacity);
        let mut out = vec![Vec::new(); rows as usize];
        let spans = self.spans.borrow();
        let no_spans = Vec::new();
        let spans = spans.as_ref().map_or(&no_spans, |(_, spans)| spans);
        for slot in 0..shown {
            let line = slot * lines / shown.max(1);
            let words = match &page {
                Some(page) => Self::page_words(&page[line], theme),
                None => self.code_words(line, spans, theme),
            };
            let row = slot / MINIMAP_LINES_PER_ROW;
            let y = ((slot % MINIMAP_LINES_PER_ROW) * 256 / MINIMAP_LINES_PER_ROW) as u8;
            for (column, size, color) in words {
                if column < MINIMAP_COLUMNS {
                    let x = (column * 256 / MINIMAP_COLUMNS) as u8;
                    let w = (size.min(MINIMAP_COLUMNS - column) * 256 / MINIMAP_COLUMNS) as u8;
                    let color = dimmed(color, theme.text.bg);
                    out[row].push(Strip { x, y, w, color });
                }
            }
        }
        out
    }

    /// Pictures in the map, small, where they stand in the document.
    fn draw_minimap_pictures(
        &self,
        grid: &mut Grid,
        track: Rect,
        (margin, width): (f32, f32),
        theme: &Theme,
    ) {
        if !self.rendered {
            return;
        }
        let area = self.text_area(Rect::new(
            track.x.saturating_sub(self.view_width.get()),
            track.y,
            self.view_width.get() + track.w,
            track.h,
        ));
        let page = self.page(area);
        let figures = self.figures();
        let per_row = MINIMAP_LINES_PER_ROW as f32;
        let capacity = track.h as f32 * per_row;
        let squeeze = (capacity / page.len().max(1) as f32).min(1.0);
        let column = width / MINIMAP_COLUMNS as f32;
        let clip = Bounds::cells(
            track.x as f32,
            track.y as f32,
            track.w as f32,
            track.h as f32,
        );
        for (index, row) in page.iter().enumerate() {
            let Some((figure, x)) = row.figure else {
                continue;
            };
            let Some(figure) = figures.get(figure) else {
                continue;
            };
            let top = track.y as f32 + index as f32 * squeeze / per_row;
            let bounds = Bounds::cells(
                track.x as f32 + margin + x * column,
                top,
                figure.width * column,
                figure.height * squeeze / per_row,
            );
            grid.paint.push(Paint::Image {
                bounds,
                clip,
                bitmap: figure.bitmap.clone(),
                color: dimmed(theme.text.fg, theme.text.bg),
            });
        }
    }

    fn draw_minimap(&self, grid: &mut Grid, full: Rect, theme: &Theme) {
        let text = self.text_area(full);
        let track = Rect::new(text.right(), full.y, full.w - text.w, full.h);
        grid.fill(track, theme.text);
        let total = if self.rendered {
            self.page(text).len()
        } else {
            self.buffer.rope().len_lines()
        }
        .max(1);
        let revision = self.buffer.revision();
        let built = (revision, track.h, text.w);
        let stale = !self
            .minimap
            .borrow()
            .as_ref()
            .is_some_and(|(key, _)| *key == built);
        if stale {
            let rows = self.minimap_rows(track.h, text, theme);
            *self.minimap.borrow_mut() = Some((built, rows));
        }
        let height = track.h as f32;
        let filled = (total as f32 / MINIMAP_LINES_PER_ROW as f32).min(height);
        let span = (text.h as f32 * filled / total as f32).clamp(1.0, height);
        // Past the fold the marker's top stops short, so the end of the file meets the end
        // of the map rather than sliding off it.
        let range = total.saturating_sub(text.h as usize).max(1) as f32;
        let travel = (filled - span).max(0.0);
        let first = if self.rendered {
            self.top
        } else {
            self.code_rows(text.w).get(self.top).map_or(0, |r| r.line)
        };
        let top = track.y as f32 + (first as f32 / range).min(1.0) * travel;
        let margin = 0.6;
        let width = track.w as f32 - margin * 1.5;
        let view = Bounds::cells(track.x as f32 + margin - 0.3, top, width + 0.6, span);
        grid.panel(view, 12, Some(theme.minimap_view), Some(theme.popup_border));
        self.draw_minimap_pictures(grid, track, (margin, width), theme);
        if let Some((_, rows)) = self.minimap.borrow().as_ref() {
            for (row, strips) in rows.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
                let bounds = Bounds::cells(
                    track.x as f32 + margin,
                    track.y as f32 + row as f32,
                    width,
                    1.0,
                );
                grid.paint.push(cellix_render::Paint::Strips {
                    bounds,
                    strips: strips.clone(),
                });
            }
        }
        // Changes down the edge, so they're findable in a long file.
        let lines = self.buffer.rope().len_lines().max(1);
        let fill_lines = lines as f32;
        for (line, change) in &self.line_changes().1 {
            let y = track.y as f32 + *line as f32 * filled / fill_lines;
            let color = match change {
                LineChange::Added => theme.git_added,
                LineChange::Modified => theme.git_modified,
                LineChange::DeletedAbove => theme.error,
            };
            let height = (filled / fill_lines).max(0.1);
            let mark = Bounds::cells(track.right() as f32 - 0.45, y, 0.3, height);
            grid.panel(mark, 0, Some(color), None);
        }
    }

    /// Scrolls to a point on the minimap or scrollbar.
    fn map_filled(&self, area: Rect, lines: usize) -> usize {
        if self.shows_minimap(area) {
            (lines.div_ceil(MINIMAP_LINES_PER_ROW)).clamp(1, area.h as usize)
        } else {
            area.h.max(1) as usize
        }
    }

    /// Pages use the whole pane, with room in the left margin for marks.
    fn page_margin(_area: Rect) -> u16 {
        PAGE_MARGIN
    }

    fn page_width(area: Rect) -> usize {
        let right = if area.w < NARROW_PANE {
            NARROW_RIGHT_MARGIN
        } else {
            PAGE_MARGIN
        };
        area.w
            .saturating_sub(Self::page_margin(area) + right + 1)
            .clamp(1, MAX_PAGE_WIDTH) as usize
    }

    /// The paper this file is part of, read again once each time the file is saved.
    fn project(&self, text: &str) -> Rc<Project> {
        let saved = self.buffer.is_dirty();
        let mut cache = self.project.borrow_mut();
        if let Some((revision, project)) = cache.as_ref()
            && (saved || *revision == self.buffer.revision())
        {
            return project.clone();
        }
        let project = match self.buffer.path() {
            Some(path) if !saved => {
                let root = cellix_doc::latex::root_of(path, text);
                Project::read(&root)
            }
            _ => Project::from_text(text),
        };
        let project = Rc::new(project);
        *cache = Some((self.buffer.revision(), project.clone()));
        project
    }

    /// Words in the document, and in the selection when there is one. A paper split over
    /// `\input` files counts whole, from its root, and the last says it did; any of its files
    /// open in `others` counts as edited there, saved or not. Counted once a revision.
    pub fn word_count(&self, others: &[&Editor]) -> (usize, Option<usize>, bool) {
        let latex = self.dialect() == Dialect::Latex;
        // Other open papers with unsaved edits, which count as they stand.
        let unsaved: Vec<&Editor> = others
            .iter()
            .copied()
            .filter(|e| latex && e.buffer.is_dirty() && e.dialect() == Dialect::Latex)
            .filter(|e| e.buffer.path().is_some() && !std::ptr::eq(*e, self))
            .collect();
        let stamp = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for editor in &unsaved {
                (editor.buffer.path(), editor.buffer.revision()).hash(&mut hasher);
            }
            hasher.finish()
        };
        let revision = (self.buffer.revision(), stamp);
        let (all, files) = match self.words.get() {
            Some((counted, words, files)) if counted == revision => (words, files),
            _ => {
                let text = self.buffer.text();
                let (words, files) = match self.buffer.path().filter(|_| latex) {
                    Some(path) => {
                        let root = self.paper_root(path, &text);
                        let texts: Vec<(&Path, String)> = unsaved
                            .iter()
                            .filter_map(|e| Some((e.buffer.path()?, e.buffer.text())))
                            .collect();
                        let mut open = vec![(path, text.as_str())];
                        open.extend(texts.iter().map(|(p, t)| (*p, t.as_str())));
                        let mut files = self.paper_files.borrow_mut();
                        cellix_doc::words::paper(&root, path, &open, &mut files)
                    }
                    None => (cellix_doc::words::count(&text, latex), 1),
                };
                self.words.set(Some((revision, words, files)));
                (words, files)
            }
        };
        let selected = self
            .selected_text()
            .filter(|t| !t.trim().is_empty())
            .map(|t| cellix_doc::words::count(&t, false));
        (all, selected, files > 1)
    }

    /// Counts again on the next look, as when another file of the paper was saved.
    pub fn recount(&self) {
        self.words.set(None);
    }

    /// The root of the paper this file is in. Finding it can read the folder, so it's found
    /// again only while the file is as saved, not on every keystroke.
    fn paper_root(&self, path: &Path, text: &str) -> PathBuf {
        let mut known = self.paper_root.borrow_mut();
        match known.as_ref() {
            Some(root) if self.buffer.is_dirty() => root.clone(),
            _ => known.insert(cellix_doc::latex::root_of(path, text)).clone(),
        }
    }

    /// How this document writes a suggestion.
    pub fn change_syntax(&self) -> cellix_doc::changes::Syntax {
        if self.dialect() == Dialect::Latex {
            cellix_doc::changes::Syntax::Latex
        } else {
            cellix_doc::changes::Syntax::Critic
        }
    }

    /// Replaces the whole text as one edit, keeping the caret where it was as far as it can.
    pub fn set_all_text(&mut self, text: &str) {
        let head = self.selections.primary().head;
        self.selections.set_single(Selection {
            anchor: 0,
            head: self.buffer.rope().len_chars(),
            goal: None,
        });
        self.buffer.replace(&mut self.selections, text);
        let end = self.buffer.rope().len_chars();
        self.selections.set_single(Selection::cursor(head.min(end)));
    }

    /// The paper this file belongs to, with its labels and citations.
    pub fn latex_project(&self) -> Rc<Project> {
        self.project(&self.buffer.text())
    }

    /// Where a clickable row leads, if there's one at this place on screen.
    pub fn row_target(&self, area: Rect, y: u16) -> Option<Target> {
        let area = self.text_area(area);
        let rows = self.page(area);
        let row = self.top + y.saturating_sub(area.y) as usize;
        rows.get(row)?.target.clone()
    }

    /// Shows or hides a paper's preamble.
    pub fn toggle_preamble(&mut self) {
        self.show_preamble = !self.show_preamble;
        *self.page.borrow_mut() = None;
    }

    pub fn dialect(&self) -> Dialect {
        let path = self.buffer.path();
        path.and_then(Dialect::of).unwrap_or_default()
    }

    /// Lays the page out, with `open` showing as source.
    fn lay_out(&self, text: &str, width: usize, open: Option<(usize, usize)>) -> LaidOut {
        let folder = self.buffer.path().and_then(Path::parent);
        let dialect = self.dialect();
        let extras = Extras {
            dialect,
            project: (dialect == Dialect::Latex).then(|| self.project(text)),
            file: self.buffer.path(),
            preamble: self.show_preamble,
            open,
            folder,
            library: Some(&self.library),
            dark: self.dark.get(),
            built: self.built.as_deref(),
            notebook: self.notebook.as_ref(),
            ..Extras::default()
        };
        let rows = rendered::layout_with(text, width, &*self.measure, &self.registry, &extras);
        self.wanted.borrow_mut().extend(extras.wanted.take());
        (
            Rc::new(rows),
            extras.figures.into_inner(),
            extras.spans.into_inner(),
            extras.preview.into_inner(),
        )
    }

    /// The rendered page for this width, laid out again only after an edit, or when the
    /// caret moves into or out of a formula.
    fn page(&self, area: Rect) -> Rc<Vec<Row>> {
        let width = Self::page_width(area);
        let revision = self.buffer.revision();
        let head = self.selections.primary().head;
        let mut cache = self.page.borrow_mut();
        let stale = cache
            .as_ref()
            .is_none_or(|c| c.revision != revision || c.width != width);
        if stale {
            let text = self.buffer.text();
            let (rows, figures, spans, _) = self.lay_out(&text, width, None);
            *cache = Some(PageCache {
                revision,
                width,
                open: None,
                rows,
                figures: Rc::new(figures),
                spans: Rc::new(spans),
                preview: None,
            });
        }
        let page = cache.as_mut().expect("laid out above");
        let open = page
            .spans
            .iter()
            .copied()
            .find(|s| s.0 < head && head < s.1);
        if open != page.open {
            let text = self.buffer.text();
            let (rows, figures, _, preview) = self.lay_out(&text, width, open);
            page.open = open;
            page.rows = rows;
            page.figures = Rc::new(figures);
            page.preview = preview;
        }
        page.rows.clone()
    }

    fn figures(&self) -> Rc<Vec<Figure>> {
        let cache = self.page.borrow();
        cache
            .as_ref()
            .map_or_else(Rc::default, |page| page.figures.clone())
    }

    /// The picture a screen row is on, of any kind: its source line and its box on the
    /// screen, as left, right, top and bottom.
    pub fn figure_on_row(&self, area: Rect, y: u16) -> Option<(usize, [f32; 4])> {
        let area = self.text_area(area);
        let rows = self.page(area);
        let figures = self.figures();
        let left = (area.x + Self::page_margin(area)) as f32;
        let row = (y as f32) + 0.5;
        for (index, found) in rows.iter().enumerate() {
            let Some((figure, x)) = found.figure else {
                continue;
            };
            let figure = figures.get(figure)?;
            let top = area.y as f32 + index as f32 - self.top as f32 + 0.2;
            if row < top || row > top + figure.height {
                continue;
            }
            let shown = [left + x, left + x + figure.width, top, top + figure.height];
            return Some((found.line, shown));
        }
        None
    }

    /// The source line of the code block a screen row is in, and the screen row its first
    /// row is on.
    pub fn code_block_on_row(&self, area: Rect, y: u16) -> Option<(usize, u16)> {
        let area = self.text_area(area);
        let rows = self.page(area);
        let index = self.top + y.checked_sub(area.y)? as usize;
        let row = rows.get(index).filter(|r| r.shaded)?;
        let first = (0..=index).rev().take_while(|n| rows[*n].shaded).last()?;
        let top = first.max(self.top) - self.top;
        Some((row.line, area.y + top as u16))
    }

    /// The image a screen row is on. Formulas and diagrams have no size of their own to change.
    fn image_on_row(&self, area: Rect, y: u16) -> Option<(usize, [f32; 4])> {
        let (line, shown) = self.figure_on_row(area, y)?;
        let text = self.line_text(line);
        let names_one = cellix_doc::figure::image_line(&text).is_some()
            || self.dialect() == Dialect::Latex && {
                let lines = line..self.buffer.rope().len_lines();
                let mut lines = lines.take(FIGURE_LINES);
                lines.any(|n| self.line_text(n).contains("\\includegraphics"))
            };
        names_one.then_some((line, shown))
    }

    /// What a comment on a picture is about, in words an agent can act on.
    pub fn figure_quote(&self, line: usize) -> String {
        let text = self.line_text(line);
        let next = || self.line_text(line + 1).trim().to_string();
        if let Some(image) = cellix_doc::figure::image_line(&text) {
            return format!("Image {}", image.path);
        }
        let trimmed = text.trim();
        if let Some(language) = trimmed.strip_prefix("```") {
            return format!("{} diagram: {}", language.trim(), next());
        }
        if trimmed.starts_with("$$") || trimmed.starts_with("\\[") {
            return format!("Formula: {}", trimmed.trim_matches('$'));
        }
        let lines = line..self.buffer.rope().len_lines();
        let named = lines.take(FIGURE_LINES).find_map(|n| {
            let source = self.line_text(n);
            let open = source.find("\\includegraphics")?;
            let brace = open + source[open..].find('{')?;
            let close = brace + source[brace..].find('}')?;
            Some(source[brace + 1..close].to_string())
        });
        match named {
            Some(path) => format!("Image {path}"),
            None => format!("Figure: {trimmed}"),
        }
    }

    /// Shows a handle on the image under the pointer.
    pub fn draw_hover(&self, grid: &mut Grid, area: Rect, hover: (u16, u16), theme: &Theme) {
        if self.rendered && !self.measure.in_cells() {
            self.draw_image_handle(grid, area, hover, theme);
        }
        if !self.rendered {
            self.draw_fold_chevrons(grid, area, hover, theme);
        }
    }

    /// The image whose right edge is under the pointer, ready to be dragged.
    pub fn image_edge(&self, area: Rect, x: f32, y: u16) -> Option<usize> {
        const REACH: f32 = 1.2;
        let (line, [_, right, ..]) = self.image_on_row(area, y)?;
        ((x - right).abs() <= REACH).then_some(line)
    }

    /// Sets an image's width from where its edge was dragged to. A picture stands in the
    /// middle of the page, so the edge is half its width from there.
    pub fn drag_image(&mut self, area: Rect, line: usize, x: f32) {
        const STEP: f32 = 5.0;
        let page = Self::page_width(self.text_area(area)) as f32;
        let middle = (area.x + Self::page_margin(self.text_area(area))) as f32 + page / 2.0;
        let share = (x - middle).max(0.0) * 2.0 / page.max(1.0) * 100.0;
        let percent = ((share / STEP).round() * STEP).clamp(STEP * 2.0, 100.0) as u32;
        let at = self.buffer.position(line, 0);
        self.selections.set_single(Selection::cursor(at));
        self.set_image_width(percent);
    }

    /// Draws the pictures in view, including those that started above it.
    fn draw_image_handle(&self, grid: &mut Grid, area: Rect, hover: (u16, u16), theme: &Theme) {
        let Some((_, [_, right, top, bottom])) = self.image_on_row(area, hover.1) else {
            return;
        };
        let middle = (top + bottom) / 2.0;
        let handle = Bounds::cells(right + 0.25, middle - 0.8, 0.3, 1.6);
        grid.panel(handle, 30, Some(theme.accent), None);
    }

    fn draw_figures(&self, grid: &mut Grid, area: Rect, rows: &[Row], theme: &Theme) {
        let figures = self.figures();
        let left = (area.x + Self::page_margin(area)) as f32;
        let end = (self.top + area.h as usize).min(rows.len());
        for (index, row) in rows.iter().enumerate().take(end) {
            let Some((figure, x)) = row.figure else {
                continue;
            };
            let Some(figure) = figures.get(figure) else {
                continue;
            };
            let top = area.y as f32 + index as f32 - self.top as f32;
            if top + figure.height < area.y as f32 {
                continue;
            }
            grid.paint.push(Paint::Image {
                bounds: Bounds::cells(left + x, top + 0.2, figure.width, figure.height),
                clip: Bounds::cells(area.x as f32, area.y as f32, area.w as f32, area.h as f32),
                bitmap: figure.bitmap.clone(),
                color: theme.text.fg,
            });
        }
    }

    /// The page as it stands, for tests to read.
    #[cfg(test)]
    pub fn page_rows(&self, area: Rect) -> Rc<Vec<Row>> {
        self.page(self.text_area(area))
    }

    /// What this page is waiting on LaTeX for.
    pub fn take_wanted(&self) -> Vec<Snippet> {
        self.wanted.take()
    }

    /// Sets the page again, now something it was waiting for has come.
    pub fn refresh_page(&mut self) {
        *self.page.borrow_mut() = None;
    }

    /// Text is measured differently from here on, so the page has to be set again.
    pub fn set_measure(&mut self, measure: Rc<dyn Measure>) {
        self.measure = measure;
        *self.page.borrow_mut() = None;
        *self.library.borrow_mut() = Library::default();
    }

    /// Moves a caret out of hidden source, so typing never lands inside a marker.
    pub fn prepare_edit(&mut self, area: Rect) {
        if !self.rendered {
            return;
        }
        let area = self.text_area(area);
        let primary = self.selections.primary();
        // Only for one caret: replacing it would end a multi-cursor edit after a letter.
        if primary.is_empty() && self.selections.len() == 1 {
            let at = rendered::snap(&self.page(area), primary.head);
            self.selections.set_single(Selection::cursor(at));
        }
    }

    /// Runs a move on the page, where rows and hidden markers change what it means.
    fn move_on_page(&mut self, area: Rect, direction: Direction, extend: bool) -> bool {
        let rows = self.page(area);
        // Start from a visible place, so a selection never takes in a hidden marker.
        let mut primary = self.selections.primary();
        if primary.is_empty() {
            let at = rendered::snap(&rows, primary.head);
            primary.anchor = at;
            primary.head = at;
        }
        let head = primary.head;
        let page = area.h.max(2) as isize - 1;
        let (_, column) = rendered::locate(&rows, head);
        // Selection keeps its goal as a whole number, so this one is in paint units.
        let goal = primary
            .goal
            .map_or(column, |g| g as f32 / units(1.0) as f32);
        let vertical = |delta| rendered::vertical(&rows, head, delta, goal);
        let (next, keep_goal) = match direction {
            Direction::Left => (rendered::step(&rows, head, false), false),
            Direction::Right => (rendered::step(&rows, head, true), false),
            Direction::Up => (vertical(-1), true),
            Direction::Down => (vertical(1), true),
            Direction::PageUp => (vertical(-page), true),
            Direction::PageDown => (vertical(page), true),
            Direction::LineStart | Direction::LineColumnZero => {
                (rendered::row_edge(&rows, head, false), false)
            }
            Direction::LineEnd => (rendered::row_edge(&rows, head, true), false),
            _ => return false,
        };
        self.selections.set_single(Selection {
            anchor: if extend { primary.anchor } else { next },
            head: next,
            goal: keep_goal.then_some(units(goal) as usize),
        });
        true
    }

    fn line_of_head(&self) -> (String, usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        (self.line_text(line), rope.line_to_char(line))
    }

    /// Enter on a page continues lists and quotes, and ends them on an empty item.
    fn enter_on_page(&mut self) {
        let (line, start) = self.line_of_head();
        let head = self.selections.primary().head;
        let at_end = head == start + line.chars().count();
        match rendered::enter_intent(&line) {
            Intent::Continue(marker) if at_end => {
                self.buffer
                    .replace(&mut self.selections, &format!("\n{marker}"));
            }
            Intent::EndList(hidden) => {
                self.buffer
                    .replace_range(&mut self.selections, start, start + hidden, "");
                self.selections.set_single(Selection::cursor(start));
            }
            _ => self.buffer.replace(&mut self.selections, "\n"),
        }
    }

    /// Keeps a page's numbered lists counting on once an edit takes an item out or puts one
    /// in, as part of that edit, so one undo takes back both. Only on a Markdown page, where
    /// the numbers are hidden and can't be fixed by hand, and not while changes are tracked.
    pub fn renumber_lists(&mut self) {
        if !self.rendered || self.tracking || self.dialect() != Dialect::Markdown {
            return;
        }
        let line = {
            let rope = self.buffer.rope();
            rope.char_to_line(self.selections.primary().head)
        };
        let text = self.buffer.text();
        let lines: Vec<&str> = text.split('\n').collect();
        // From the bottom up, so what's above keeps its place.
        for (index, digits, number) in rendered::renumbering(&lines, line).into_iter().rev() {
            let indent = lines[index].chars().count() - lines[index].trim_start().chars().count();
            let start = self.buffer.rope().line_to_char(index) + indent;
            let range = (start, start + digits);
            if !self
                .buffer
                .amend(&mut self.selections, range, &number.to_string())
            {
                return;
            }
        }
    }

    /// Backspace at the start of an item or heading removes its marker in one step.
    fn backspace_on_page(&mut self) {
        let (line, start) = self.line_of_head();
        let hidden = rendered::hidden_prefix(&line);
        let primary = self.selections.primary();
        if primary.is_empty() && hidden > 0 && primary.head == start + hidden {
            self.buffer
                .replace_range(&mut self.selections, start, start + hidden, "");
            self.selections.set_single(Selection::cursor(start));
        } else {
            self.buffer.delete_backward(&mut self.selections);
        }
    }

    /// Tab moves between table cells, and otherwise indents.
    /// The next empty `{}` of a formula being edited, for Tab.
    fn next_slot(&self, head: usize) -> Option<usize> {
        let cache = self.page.borrow();
        let (_, end) = cache.as_ref()?.open?;
        let rope = self.buffer.rope();
        let text: Vec<char> = rope
            .slice(head..end.min(rope.len_chars()))
            .chars()
            .collect();
        let found = text.windows(2).skip(1).position(|w| w == ['{', '}'])?;
        Some(head + found + 2)
    }

    fn tab_on_page(&mut self, area: Rect) {
        let rows = self.page(area);
        let head = self.selections.primary().head;
        if let Some(slot) = self.next_slot(head) {
            self.selections.set_single(Selection::cursor(slot));
            return;
        }
        if let Some(cell) = rendered::next_cell(&rows, head) {
            self.select_cell(area, cell);
            return;
        }
        // Past the last cell comes a new row, as in a sheet.
        if self.selection_in_table(area) {
            self.edit_table(TableEdit::RowBelow);
            let rows = self.page(area);
            let head = self.selections.primary().head;
            if let Some(cell) = rendered::cell_in_row(&rows, head, 0) {
                self.select_cell(area, cell);
            }
            return;
        }
        let (line, start) = self.line_of_head();
        if rendered::hidden_prefix(&line) > 0 {
            self.buffer
                .replace_range(&mut self.selections, start, start, "  ");
            self.selections.set_single(Selection::cursor(head + 2));
        } else {
            self.buffer.type_text(&mut self.selections, "    ");
        }
    }

    /// Whether the caret is in a table the page is showing.
    fn selection_in_table(&self, area: Rect) -> bool {
        let rows = self.page(area);
        let head = self.selections.primary().head;
        rendered::cell_column(&rows, head).is_some() && self.table_at_caret().is_some()
    }

    /// Lands on a cell with what it holds selected, so typing replaces it.
    fn select_cell(&mut self, area: Rect, cell: usize) {
        let rows = self.page(area);
        let (anchor, head) = rendered::cell_range(&rows, cell);
        self.selections.set_single(Selection {
            anchor,
            head,
            goal: None,
        });
    }

    /// Enter in a table: the same column one row down, with a new row past the last.
    fn next_row(&mut self, area: Rect) {
        let rows = self.page(area);
        let head = self.selections.primary().start();
        let column = rendered::cell_column(&rows, head).unwrap_or(0);
        if let Some(cell) = rendered::cell_below(&rows, head) {
            self.select_cell(area, cell);
            return;
        }
        self.edit_table(TableEdit::RowBelow);
        let rows = self.page(area);
        let head = self.selections.primary().head;
        if let Some(cell) = rendered::cell_in_row(&rows, head, column) {
            self.select_cell(area, cell);
        }
    }

    /// Toggles a task checkbox at a click. Returns true if there was one.
    pub fn click_checkbox(&mut self, area: Rect, x: f32, y: u16) -> bool {
        if !self.rendered {
            return false;
        }
        let area = self.text_area(area);
        let rows = self.page(area);
        let row = self.top + y.saturating_sub(area.y) as usize;
        let column = x - (area.x + Self::page_margin(area)) as f32;
        let Some((at, mark)) = rows.get(row).and_then(|r| r.checkbox) else {
            return false;
        };
        if column < at || column >= at + 1.3 {
            return false;
        }
        let checked = self.buffer.rope().char(mark) != ' ';
        let text = if checked { " " } else { "x" };
        self.buffer
            .replace_range(&mut self.selections, mark, mark + 1, text);
        true
    }

    /// True when a click at `x` lands on the scrollbar.
    pub fn on_scrollbar(&self, area: Rect, x: u16) -> bool {
        x >= self.text_area(area).right()
    }

    /// Scrolls so that row `y` of the scrollbar's track matches the document.
    pub fn scroll_to_row(&mut self, area: Rect, y: u16) {
        let lines = if self.rendered {
            self.page(self.text_area(area)).len()
        } else {
            self.buffer.rope().len_lines()
        };
        let row = y.saturating_sub(area.y) as usize;
        if !self.rendered {
            // The map and the bar are drawn by line; the view scrolls by row.
            let filled = self.map_filled(area, lines);
            let line = (row * lines / filled).min(lines.saturating_sub(1));
            let rows = self.code_rows(self.text_area(area).w);
            let middle = rows.row_of(line, 0).saturating_sub(area.h as usize / 2);
            self.view_rows.set(area.h);
            self.top = middle.min(self.last_top());
            return;
        }
        let filled = self.map_filled(area, lines);
        // The point clicked comes to the middle of the view.
        let middle = (row * lines / filled).saturating_sub(area.h as usize / 2);
        self.view_rows.set(area.h);
        self.top = middle.min(self.last_top());
    }

    /// Where code goes in a pane, and how wide its line numbers are, for what is drawn over
    /// the code, as merge conflicts are.
    pub fn code_layout(&self, area: Rect) -> (Rect, u16) {
        let text = self.text_area(area);
        (text, self.gutter_width(text.w))
    }

    /// The screen rows a line's rows stand on in the code view, for what's drawn over them:
    /// none where it's folded away, scrolled off, or under a pinned header.
    pub fn line_rows_on_screen(&self, area: Rect, line: usize) -> Vec<u16> {
        let text = self.text_area(area);
        let rows = self.code_rows(text.w);
        let pinned = self.sticky(&rows, text).len();
        rows.rows_of(line)
            .filter_map(|row| {
                let offset = row.checked_sub(self.top)?;
                (offset >= pinned && offset < text.h as usize).then(|| text.y + offset as u16)
            })
            .collect()
    }

    /// Narrow panes keep only the digits they need: every column counts there.
    fn gutter_width(&self, pane_width: u16) -> u16 {
        let digits = self.buffer.rope().len_lines().to_string().len();
        let minimum = if pane_width < NARROW_PANE {
            2
        } else {
            GUTTER_MIN_DIGITS
        };
        // A pad, the number, the cells for a fold's chevron, and a space before the text.
        (digits.max(minimum) + 2) as u16 + Self::chevron_cells(pane_width)
    }

    /// Two cells for fold chevrons, except in a narrow pane, where every column counts and
    /// folds are left to the keys and the palette.
    fn chevron_cells(pane_width: u16) -> u16 {
        if pane_width < NARROW_PANE { 0 } else { 2 }
    }

    /// How wide the line number is drawn, right-aligned, its pad included.
    fn number_width(&self, pane_width: u16) -> usize {
        (self.gutter_width(pane_width) - 1 - Self::chevron_cells(pane_width)) as usize
    }

    pub fn line_text(&self, line: usize) -> String {
        let text = self.buffer.rope().line(line).to_string();
        text.trim_end_matches(['\n', '\r']).to_string()
    }

    /// Takes the server's hints, as (line, column, label), for this revision.
    pub fn set_hints(&mut self, found: Vec<(usize, usize, String)>) {
        let rope = self.buffer.rope();
        let mut hints: Vec<(usize, String)> = found
            .into_iter()
            .filter(|(line, _, _)| *line < rope.len_lines())
            .map(|(line, column, label)| (self.buffer.position(line, column), label))
            .collect();
        hints.sort_by_key(|(at, _)| *at);
        self.hints = hints;
        self.hints_for = (rope.len_chars(), self.buffer.revision());
    }

    pub fn clear_hints(&mut self) {
        self.hints.clear();
        self.hints_asked = None;
    }

    /// Moves the hints with an edit made since they came: those after it by what it added
    /// or took away, and those in text it took away go.
    pub fn carry_hints(&mut self) {
        let (length, revision) = self.hints_for;
        if revision == self.buffer.revision() {
            return;
        }
        let now = self.buffer.rope().len_chars();
        let moved = now as isize - length as isize;
        let head = self.selections.primary().head as isize;
        let pivot = head - moved;
        self.hints
            .retain(|(at, _)| moved >= 0 || (*at as isize) < head || (*at as isize) >= pivot);
        for (at, _) in &mut self.hints {
            if *at as isize >= pivot {
                *at = (*at as isize + moved).clamp(0, now as isize) as usize;
            }
        }
        self.hints_for = (now, self.buffer.revision());
    }

    /// The hints at places `from..to`.
    fn hints_in(&self, from: usize, to: usize) -> &[(usize, String)] {
        let start = self.hints.partition_point(|(at, _)| *at < from);
        let end = self.hints.partition_point(|(at, _)| *at < to);
        &self.hints[start..end.max(start)]
    }

    fn hints_width(&self, from: usize, to: usize) -> usize {
        self.hints_in(from, to)
            .iter()
            .map(|(_, label)| self.hint_cells(label).2)
            .sum()
    }

    /// How a hint sits in the line: where its chip starts, how wide the chip is, and the
    /// cells it takes in all. On cells a letter or a space of padding is a cell. With pixels
    /// the chip is as wide as its smaller text needs, so the letters keep their own spacing,
    /// and padding is a sliver beside it rather than a whole cell.
    fn hint_cells(&self, label: &str) -> (f32, f32, usize) {
        const PADDING: f32 = 0.3;
        let lead = label.chars().take_while(|c| *c == ' ').count();
        let body = label.trim();
        let trail = label.chars().count() - lead - body.chars().count();
        if self.measure.in_cells() || body.is_empty() {
            let letters = body.chars().count();
            return (lead as f32, letters as f32, lead + letters + trail);
        }
        let before = PADDING * lead.min(1) as f32;
        let after = PADDING * trail.min(1) as f32;
        let chip = self.measure.width(body, Self::HINT_FONT) + 0.8;
        let total = (before + chip + after).ceil();
        // What rounding up leaves goes to the chip, so the code after it stays close.
        (before, total - before - after, total as usize)
    }

    const HINT_FONT: cellix_render::Font = cellix_render::Font {
        mono: true,
        bold: false,
        italic: false,
        size: 85,
    };

    /// Display column of a char position, counting the hints before it on its line.
    fn column_of(&self, at: usize) -> usize {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(at);
        let start = rope.line_to_char(line);
        let mut column = 0;
        for c in rope.slice(start..at).chars() {
            column += display_width(c, column, self.indentation.size);
        }
        column + self.hints_width(start, at)
    }

    /// Char position at a screen cell inside `area`, clamped to the text.
    pub fn position_at(&self, area: Rect, fine_x: f32, y: u16) -> usize {
        let x = fine_x as u16;
        let area = self.text_area(area);
        if self.rendered {
            let rows = self.page(area);
            let row = self.top + y.saturating_sub(area.y) as usize;
            let column = (fine_x - (area.x + Self::page_margin(area)) as f32).max(0.0);
            let row = row.min(rows.len().saturating_sub(1));
            // A tall heading's lower half is the spacer under it, and still the heading.
            let above = row
                .checked_sub(1)
                .and_then(|r| rows.get(r))
                .filter(|r| r.tall);
            // So is any part of a picture, which opens it wherever it's clicked.
            let picture = (!rows[row].stop)
                .then(|| {
                    rows[..=row]
                        .iter()
                        .rev()
                        .find(|r| r.stop || r.figure.is_some())
                })
                .flatten()
                .filter(|r| r.figure.is_some());
            let Some(special) = above.or(picture) else {
                // A row of a table is asked with the rows around it: under a cell whose
                // text has ended, the place is that text's end.
                let stop = (row..rows.len()).find(|index| rows[*index].stop);
                return stop.map_or(self.buffer.rope().len_chars(), |index| {
                    rendered::position_in(&rows, index, column)
                });
            };
            return special.position_at(column);
        }
        let rows = self.code_rows(area.w);
        let index = (self.top + (y.saturating_sub(area.y)) as usize).min(rows.len() - 1);
        let row = rows.get(index).expect("a row in range");
        let target = self.left() + x.saturating_sub(area.x + self.gutter_width(area.w)) as usize;
        self.row_char_at(row, target)
    }

    /// A folded region's ⋯ after its header's text, then the bracket it took in.
    fn draw_fold_chip(
        &self,
        grid: &mut Grid,
        (x, y): (u16, u16),
        right: u16,
        line: usize,
        (text_style, theme): (cellix_render::Style, &Theme),
    ) {
        if x + 3 > right {
            return;
        }
        let chip = cellix_render::Style {
            fg: theme.text.fg,
            bg: theme.fold_chip(),
            painted: !self.measure.in_cells(),
            ..text_style
        };
        grid.put_str(x, y, " ⋯ ", chip, right);
        if !self.measure.in_cells() {
            // One rounded shape, clear of the braces either side, lighter than both the
            // line and the caret's line.
            let pill = Bounds::cells(x as f32 + 0.2, y as f32 + 0.14, 2.6, 0.72);
            grid.panel(pill, 22, Some(theme.fold_chip()), None);
            grid.centred_text(x as f32 + 1.5, y as f32, "⋯", theme.text.fg, 100);
            // Under paint the cells stay the line's colour, so only the pill shows.
            grid.set_style(Rect::new(x, y, 3, 1), |style| cellix_render::Style {
                bg: text_style.bg,
                ..style
            });
        }
        let rope = self.buffer.rope();
        let range = self.hidden().into_iter().find(|r| r.start == line);
        let Some(range) = range else {
            return;
        };
        let Some(closer) = fold::closer(rope, range) else {
            return;
        };
        grid.put_str(x + 3, y, &closer, text_style, right);
        // Its brackets keep their pair's colour, as they would unfolded.
        if self.bracket_pairs {
            let found = self.pairs_found();
            let start = rope.line_to_char(range.end);
            let indent = self
                .line_text(range.end)
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            let colours = theme.brackets();
            for bracket in brackets::between(
                &found,
                start + indent,
                start + indent + closer.chars().count(),
            ) {
                let fg = match bracket.partner {
                    Some(_) => colours[bracket.depth % colours.len()],
                    None => theme.error,
                };
                let at = x + 3 + (bracket.at - start - indent) as u16;
                if at < right {
                    grid.set_style(Rect::new(at, y, 1, 1), |style| cellix_render::Style {
                        fg,
                        ..style
                    });
                }
            }
        }
    }

    /// Where a click in the gutter or on a fold's chip opens or closes a fold. True when it
    /// did.
    pub fn click_fold(&mut self, area: Rect, x: f32, y: u16) -> bool {
        let Some(line) = self.fold_spot(area, x, y) else {
            return false;
        };
        let text = self.text_area(area);
        let pinned = self.sticky(&self.code_rows(text.w), text);
        let outside = pinned.iter().position(|l| *l == line);
        self.toggle_fold(line);
        if let Some(outside) = outside {
            // A pinned header folds where it stands, so the view goes back up to it.
            let row = self.code_rows(text.w).row_of(line, 0);
            self.view_rows.set(text.h);
            self.top = row.saturating_sub(outside).min(self.last_top());
        }
        true
    }

    /// The header line of the fold a click at this place would open or close: a chevron in
    /// the gutter, or a folded region's chip.
    pub fn fold_spot(&self, area: Rect, x: f32, y: u16) -> Option<usize> {
        if self.rendered {
            return None;
        }
        let text = self.text_area(area);
        let gutter = self.gutter_width(text.w);
        let rows = self.code_rows(text.w);
        let offset = y.checked_sub(text.y)? as usize;
        let cell = x as u16;
        let cells = Self::chevron_cells(text.w);
        let first = text.x + gutter - 1 - cells;
        let chevrons = first..first + cells;
        // Over a pinned header only its chevron folds; the rest of it jumps there.
        if let Some(line) = self.sticky(&rows, text).get(offset) {
            return chevrons.contains(&cell).then_some(*line);
        }
        let row = rows.get(self.top + offset)?;
        let folded = self.fold_at(row.line)?;
        if chevrons.contains(&cell) && row.first() {
            return Some(row.line);
        }
        let last = rows.rows_of(row.line).last().and_then(|r| rows.get(r));
        if !folded || last != Some(row) {
            return None;
        }
        let start = self.buffer.rope().line_to_char(row.line);
        let line_end = start + self.line_text(row.line).chars().count();
        let column = self.row_column(row, line_end).checked_sub(self.left())?;
        let chip = text.x as usize + gutter as usize + column;
        (chip..chip + 3)
            .contains(&(cell as usize))
            .then_some(row.line)
    }

    /// Chevrons beside every region that can fold, while the pointer is over the gutter.
    fn draw_fold_chevrons(&self, grid: &mut Grid, area: Rect, hover: (u16, u16), theme: &Theme) {
        let text = self.text_area(area);
        let gutter = self.gutter_width(text.w);
        if hover.0 >= text.x + gutter || hover.0 < text.x {
            return;
        }
        if Self::chevron_cells(text.w) == 0 {
            return;
        }
        let rows = self.code_rows(text.w);
        let current = self
            .buffer
            .rope()
            .char_to_line(self.selections.primary().head);
        let pinned = self.sticky(&rows, text);
        for offset in 0..text.h {
            // Pinned headers fold too, as VS Code's do.
            let line = match pinned.get(offset as usize) {
                Some(line) => *line,
                None => match rows.get(self.top + offset as usize) {
                    Some(row) if row.first() => row.line,
                    Some(_) => continue,
                    None => break,
                },
            };
            let Some(folded) = self.fold_at(line) else {
                continue;
            };
            let y = text.y + offset;
            let bg = if line == current && (offset as usize) >= pinned.len() {
                theme.line
            } else {
                theme.text.bg
            };
            let style = cellix_render::Style { bg, ..theme.gutter };
            let icon = if folded {
                Icon::ChevronRight
            } else {
                Icon::ChevronDown
            };
            self.draw_chevron(grid, (text.x + gutter, y), icon, (style, theme));
        }
    }

    /// A fold's chevron, in its own lane between the line number and the code, drawn in the
    /// text's colour so it doesn't read as part of the number.
    fn draw_chevron(
        &self,
        grid: &mut Grid,
        (code, y): (u16, u16),
        icon: Icon,
        (style, theme): (cellix_render::Style, &Theme),
    ) {
        let fallback = if icon == Icon::ChevronRight {
            "›"
        } else {
            "⌄"
        };
        let style = cellix_render::Style {
            fg: theme.text.fg,
            ..style
        };
        let x = (code - 3) as f32 + if self.measure.in_cells() { 0.0 } else { 0.3 };
        grid.glyph(x, y, icon, theme.text.fg, fallback, style);
    }

    /// The header lines of the scopes the view has scrolled into, outermost first, that stay
    /// pinned over the top rows. The line they're worked out for is the first one under them.
    fn sticky(&self, rows: &Rows, area: Rect) -> Vec<usize> {
        let most = self.sticky_lines.min(area.h as usize / 3);
        if most == 0 || self.top == 0 || self.rendered {
            return Vec::new();
        }
        // A scope's closing bracket isn't inside it here: with only the `}` left in view,
        // pinning its header above it would say nothing, and VS Code doesn't.
        let mut ranges: Vec<fold::Range> = self
            .fold_ranges()
            .iter()
            .map(|&r| match fold::closer(self.buffer.rope(), r) {
                Some(_) => fold::Range {
                    end: r.end - 1,
                    ..r
                },
                None => r,
            })
            .collect();
        ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
        // Outermost first, each pinned at the next slot while its header has scrolled up past
        // that slot and its body still shows below it.
        let mut found: Vec<fold::Range> = Vec::new();
        for range in ranges {
            if found.len() == most {
                break;
            }
            if found.last().is_some_and(|outer| range.end > outer.end) {
                continue;
            }
            let slot = self.top + found.len();
            if rows.row_of(range.start, 0) < slot && rows.row_of(range.end, usize::MAX) > slot {
                found.push(range);
            }
        }
        found.into_iter().map(|r| r.start).collect()
    }

    /// The line a pinned header at row `y` jumps to, when one is there.
    pub fn sticky_at(&self, area: Rect, y: u16) -> Option<usize> {
        if self.rendered {
            return None;
        }
        let text = self.text_area(area);
        let rows = self.code_rows(text.w);
        let pinned = self.sticky(&rows, text);
        pinned.get(y.checked_sub(text.y)? as usize).copied()
    }

    /// Brings a pinned header's line up under the headers outside it, with the caret at its
    /// first word, so its scope shows from the start.
    pub fn jump_to_sticky(&mut self, area: Rect, line: usize) {
        let text = self.text_area(area);
        let rows = self.code_rows(text.w);
        let outside = self
            .sticky(&rows, text)
            .iter()
            .position(|l| *l == line)
            .unwrap_or(0);
        let indent = self
            .line_text(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let at = self.buffer.rope().line_to_char(line) + indent;
        self.selections.set_single(Selection::cursor(at));
        self.top = rows.row_of(line, 0).saturating_sub(outside);
        self.view_rows.set(text.h);
        self.top = self.top.min(self.last_top());
        self.reveal(area);
    }

    /// The pinned headers over the top rows: each line's number and text in its colours, and
    /// a soft shadow under them on pixels, a line under them on cells.
    fn draw_sticky(
        &self,
        grid: &mut Grid,
        area: Rect,
        (gutter, rows): (u16, &Rows),
        theme: &Theme,
    ) {
        let pinned = self.sticky(rows, area);
        if pinned.is_empty() {
            return;
        }
        let rope = self.buffer.rope();
        let found = self.bracket_pairs.then(|| self.pairs_found());
        let spans = self.spans.borrow();
        let spans = spans
            .as_ref()
            .filter(|(r, _)| *r == self.buffer.revision())
            .map(|(_, s)| s.as_slice());
        for (n, line) in pinned.iter().enumerate() {
            let y = area.y + n as u16;
            grid.fill(Rect::new(area.x, y, area.w, 1), theme.text);
            let number = format!("{:>w$}", line + 1, w = self.number_width(area.w));
            grid.put_str(area.x, y, &number, theme.gutter, area.x + gutter);
            let row = CodeRow {
                line: *line,
                start: 0,
                end: usize::MAX,
                indent: 0,
            };
            let start = rope.line_to_char(*line);
            let mut text = String::new();
            let mut column = 0;
            for c in self.line_text(*line).chars() {
                let width = display_width(c, column, self.indentation.size);
                if column + width > self.left() {
                    if c == '\t' {
                        text.extend(std::iter::repeat_n(' ', width));
                    } else {
                        text.push(c);
                    }
                }
                column += width;
            }
            grid.put_str(area.x + gutter, y, &text, theme.text, area.right());
            let colour = |grid: &mut Grid, from: usize, to: usize, fg: Rgb| {
                let x_from = self.row_column(row, from).saturating_sub(self.left());
                let x_to = self.row_column(row, to).saturating_sub(self.left());
                let x = (area.x + gutter).saturating_add(x_from as u16);
                let w = (x_to.saturating_sub(x_from) as u16).min(area.right().saturating_sub(x));
                grid.set_style(Rect::new(x, y, w, 1), |style| cellix_render::Style {
                    fg,
                    ..style
                });
            };
            let end = start + self.line_text(*line).chars().count();
            if let Some(spans) = spans {
                let (from, to) = (rope.char_to_byte(start), rope.char_to_byte(end));
                let first = spans.partition_point(|s| s.end <= from);
                for span in spans[first..].iter().take_while(|s| s.start < to) {
                    let a = rope.byte_to_char(span.start.max(from));
                    let b = rope.byte_to_char(span.end.min(to));
                    colour(grid, a, b, theme.token(span.token));
                }
            }
            if let Some(found) = &found {
                let colours = theme.brackets();
                for bracket in brackets::between(found, start, end) {
                    let fg = if bracket.partner.is_some() {
                        colours[bracket.depth % colours.len()]
                    } else {
                        theme.error
                    };
                    colour(grid, bracket.at, bracket.at + 1, fg);
                }
            }
        }
        let last = area.y + pinned.len() as u16 - 1;
        if grid.cursor.is_some_and(|(_, y)| y <= last) {
            grid.cursor = None;
        }
        if self.measure.in_cells() {
            grid.set_style(Rect::new(area.x, last, area.w, 1), |style| {
                cellix_render::Style {
                    underline: true,
                    ..style
                }
            });
        } else {
            // A shadow fading down from under the last pinned line.
            // About 4 px, so it ends above the next row's glyphs. Light themes take less, so
            // it stays a shadow and not the heaviest line there.
            let below = (last + 1) as f32;
            let alphas = if theme.is_light() {
                [34, 18, 8]
            } else {
                [70, 40, 18]
            };
            for (n, alpha) in alphas.into_iter().enumerate() {
                let band =
                    Bounds::cells(area.x as f32, below + n as f32 * 0.04, area.w as f32, 0.04);
                grid.wash(band, 0, (Rgb(0, 0, 0), alpha), None);
            }
        }
    }

    /// How far the code is scrolled sideways: never while it wraps.
    fn left(&self) -> usize {
        if self.wrap { 0 } else { self.left }
    }

    /// The rows the code shows in a text area `width` wide: a line each, unless lines wrap
    /// or fold.
    fn code_rows(&self, width: u16) -> Rc<Rows> {
        let room = width.saturating_sub(self.gutter_width(width)) as usize;
        // One column stays free for the caret after a full row.
        let wrap = (self.wrap && width > 0).then(|| room.saturating_sub(1).max(8));
        let key = (self.buffer.revision(), wrap, self.folds_stamp());
        if let Some((cached, rows)) = self.rows.borrow().as_ref()
            && *cached == key
        {
            return rows.clone();
        }
        let hidden = self.hidden();
        let rows = Rc::new(Rows::lay_out(
            self.buffer.rope(),
            &hidden,
            wrap,
            self.indentation.size,
        ));
        *self.rows.borrow_mut() = Some((key, rows.clone()));
        rows
    }

    /// The rows as last drawn, for what has no area to hand.
    fn shown_rows(&self) -> Rc<Rows> {
        self.code_rows(self.view_width.get())
    }

    /// Lines folded away, as regions in order.
    fn hidden(&self) -> Vec<fold::Range> {
        self.follow_folds();
        let folded = self.folded.borrow();
        if folded.headers.is_empty() {
            return Vec::new();
        }
        let ranges = self.fold_ranges();
        let mut hidden: Vec<fold::Range> = folded
            .headers
            .iter()
            .filter_map(|(line, _)| {
                ranges
                    .iter()
                    .filter(|r| r.start == *line)
                    .max_by_key(|r| r.end)
                    .copied()
            })
            .collect();
        hidden.sort();
        hidden
    }

    /// Where the code can fold: the server's regions once it has said, else the indent's.
    pub fn fold_ranges(&self) -> Rc<Vec<fold::Range>> {
        let answers = self
            .server_folds
            .as_ref()
            .map_or(0, |(answers, _)| *answers);
        let key = (self.buffer.revision(), answers);
        if let Some((cached, ranges)) = self.fold_ranges.borrow().as_ref()
            && *cached == key
        {
            return ranges.clone();
        }
        let rope = self.buffer.rope();
        let ranges = match &self.server_folds {
            _ if self.rendered || rope.len_bytes() > brackets::MAX_BYTES => Vec::new(),
            Some((_, server)) if !server.is_empty() => fold::from_server(rope, server),
            _ => fold::by_indent(rope, self.indentation.size),
        };
        let ranges = Rc::new(ranges);
        *self.fold_ranges.borrow_mut() = Some((key, ranges.clone()));
        ranges
    }

    /// Takes the language server's regions, in place of the indent's.
    pub fn set_server_folds(&mut self, ranges: Vec<(usize, usize)>) {
        let answers = self
            .server_folds
            .as_ref()
            .map_or(0, |(answers, _)| *answers);
        self.server_folds = Some((answers + 1, ranges));
    }

    /// Keeps each fold on its header line after an edit: where the line moved to, found by
    /// its text near where it was, or gone with it.
    fn follow_folds(&self) {
        let rope = self.buffer.rope();
        let revision = self.buffer.revision();
        let mut folded = self.folded.borrow_mut();
        if folded.revision == revision {
            return;
        }
        let lines = rope.len_lines();
        let moved = lines as isize - folded.lines as isize;
        let reach = moved.unsigned_abs() + 2;
        let text_of = |line: usize| (line < lines).then(|| self.line_text(line));
        let headers = std::mem::take(&mut folded.headers)
            .into_iter()
            .filter_map(|(line, text)| {
                // Where it was, where the change in length puts it, then nearer first.
                let shifted = line.saturating_add_signed(moved);
                let around = (1..=reach)
                    .flat_map(|d| [shifted.checked_sub(d), Some(shifted + d)])
                    .flatten();
                let found = [line, shifted]
                    .into_iter()
                    .chain(around)
                    .find(|l| text_of(*l).as_deref() == Some(text.as_str()));
                found.map(|l| (l, text))
            })
            .collect();
        *folded = Folded {
            revision,
            lines,
            headers,
        };
    }

    /// Whether a line heads a region that can fold, and whether it's folded.
    fn fold_at(&self, line: usize) -> Option<bool> {
        let ranges = self.fold_ranges();
        ranges.iter().any(|r| r.start == line).then(|| {
            self.follow_folds();
            self.folded.borrow().headers.iter().any(|(l, _)| *l == line)
        })
    }

    /// Folds the region headed by `line`, or opens it.
    pub fn toggle_fold(&mut self, line: usize) {
        match self.fold_at(line) {
            Some(true) => self.unfold_lines(|start, _| start == line),
            Some(false) => self.fold_lines(&[line]),
            None => {}
        }
    }

    fn fold_lines(&mut self, lines: &[usize]) {
        self.follow_folds();
        {
            let mut folded = self.folded.borrow_mut();
            for line in lines {
                if !folded.headers.iter().any(|(l, _)| l == line) {
                    folded.headers.push((*line, self.line_text(*line)));
                }
            }
        }
        // A caret inside goes to the end of the line the fold stands on.
        let rope = self.buffer.rope();
        let hidden = self.hidden();
        let moved: Vec<Selection> = self
            .selections
            .iter()
            .map(|s| {
                let line = rope.char_to_line(s.head);
                match hidden.iter().find(|r| r.start < line && line <= r.end) {
                    Some(r) => Selection::cursor(
                        rope.line_to_char(r.start) + self.line_text(r.start).chars().count(),
                    ),
                    None => *s,
                }
            })
            .collect();
        self.selections.replace_all(moved);
    }

    /// Opens every fold whose region, as (header, last line), passes `which`.
    fn unfold_lines(&mut self, which: impl Fn(usize, usize) -> bool) {
        let hidden = self.hidden();
        let open: Vec<usize> = hidden
            .iter()
            .filter(|r| which(r.start, r.end))
            .map(|r| r.start)
            .collect();
        self.folded
            .borrow_mut()
            .headers
            .retain(|(line, _)| !open.contains(line));
    }

    /// Opens any fold a caret has landed inside, as after a search or a jump.
    fn open_folds_at_carets(&mut self) {
        if self.folded.borrow().headers.is_empty() {
            return;
        }
        let rope = self.buffer.rope();
        let lines: Vec<usize> = self
            .selections
            .iter()
            .flat_map(|s| [s.head, s.anchor])
            .map(|at| rope.char_to_line(at.min(rope.len_chars())))
            .collect();
        loop {
            let before = self.folded.borrow().headers.len();
            self.unfold_lines(|start, end| lines.iter().any(|l| start < *l && *l <= end));
            if self.folded.borrow().headers.len() == before {
                break;
            }
        }
    }

    /// The fold commands: the region at the caret, or all of them.
    fn run_fold(&mut self, command: Command) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        let ranges = self.fold_ranges();
        match command {
            Command::Fold => {
                self.follow_folds();
                let folded: Vec<usize> = self
                    .folded
                    .borrow()
                    .headers
                    .iter()
                    .map(|(l, _)| *l)
                    .collect();
                // The region this line heads, or else the innermost one around it.
                let open = |r: &&fold::Range| !folded.contains(&r.start);
                let heads = ranges.iter().filter(|r| r.start == line).find(open);
                let around = ranges
                    .iter()
                    .filter(|r| r.start < line && line <= r.end)
                    .filter(open)
                    .max_by_key(|r| r.start);
                if let Some(range) = heads.or(around) {
                    self.fold_lines(&[range.start]);
                }
            }
            Command::Unfold => {
                let heads = self.hidden().iter().any(|r| r.start == line);
                if heads {
                    self.unfold_lines(|start, _| start == line);
                } else {
                    self.unfold_lines(|start, end| start < line && line <= end);
                }
            }
            Command::FoldAll => {
                let all: Vec<usize> = ranges.iter().map(|r| r.start).collect();
                self.fold_lines(&all);
            }
            Command::UnfoldAll => self.folded.borrow_mut().headers.clear(),
            _ => {}
        }
    }

    /// A stamp of the folds and where the code can fold, so rows are laid out again when
    /// either changes.
    fn folds_stamp(&self) -> u64 {
        self.follow_folds();
        let folded = self.folded.borrow();
        if folded.headers.is_empty() {
            return 0;
        }
        let answers = self
            .server_folds
            .as_ref()
            .map_or(0, |(answers, _)| *answers as u64);
        folded.headers.iter().fold(answers, |key, (line, _)| {
            key.wrapping_mul(31).wrapping_add(*line as u64 + 1)
        })
    }

    /// The column a row shows the char at `at` in, counting a wrapped row's indent.
    fn row_column(&self, row: CodeRow, at: usize) -> usize {
        let start = self.buffer.rope().line_to_char(row.line);
        let base = if row.first() {
            0
        } else {
            self.column_of(start + row.start)
        };
        row.indent as usize + self.column_of(at).saturating_sub(base)
    }

    /// The char a row shows at column `target`, or its last place when the row is shorter.
    fn row_char_at(&self, row: CodeRow, target: usize) -> usize {
        let start = self.buffer.rope().line_to_char(row.line);
        let text: Vec<char> = self.line_text(row.line).chars().collect();
        let end = row.end.min(text.len());
        let from = row.start.min(end);
        let mut column = if row.first() {
            0
        } else {
            self.column_of(start + from)
        };
        let target = target
            .saturating_sub(row.indent as usize)
            .saturating_add(column);
        let mut hinted = 0;
        for (i, c) in text[from..end].iter().enumerate() {
            // A click on a hint lands beside it, where its place is.
            let at = start + from + i;
            let hint = self.hints_width(at, at + 1);
            if column + hinted + hint > target {
                return at;
            }
            hinted += hint;
            let width = display_width(*c, column, self.indentation.size);
            if column + hinted + width > target {
                return start + from + i;
            }
            column += width;
        }
        // A wrapped row ends before the char the next row starts with.
        if row.end != usize::MAX && end > from {
            start + end - 1
        } else {
            start + end
        }
    }

    /// The row the char at `at` is drawn on, and its column there.
    fn place_of(&self, rows: &Rows, at: usize) -> (usize, usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(at);
        let index = rows.row_of(line, at - rope.line_to_char(line));
        match rows.get(index) {
            Some(row) if row.line == line => (index, self.row_column(row, at)),
            _ => (index, 0),
        }
    }

    /// The pieces of chars `from..to`, all on one line, as they're drawn: a row in view and
    /// the columns there, from the text's left edge.
    fn pieces(&self, area: Rect, rows: &Rows, from: usize, to: usize) -> Vec<(u16, usize, usize)> {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(from);
        let start = rope.line_to_char(line);
        let len = self.line_text(line).chars().count();
        let left = self.left();
        let mut out = Vec::new();
        for index in rows.rows_of(line) {
            if index < self.top || index >= self.top + area.h as usize {
                continue;
            }
            let row = rows.get(index).expect("a row of the line");
            let (a, b) = (start + row.start, start + row.end.min(len));
            let (f, t) = (from.max(a), to.min(b));
            if f > t {
                continue;
            }
            let x_from = self.row_column(row, f).saturating_sub(left);
            let x_to = self.row_column(row, t).saturating_sub(left);
            out.push((area.y + (index - self.top) as u16, x_from, x_to));
        }
        out
    }

    /// The first line shown and the line after the last, with `rows` rows of code in view.
    pub fn shown_lines(&self, rows: usize) -> (usize, usize) {
        let laid = self.shown_rows();
        let line = |row: usize| laid.get(row).map(|r| r.line);
        let first = line(self.top).unwrap_or(0);
        let after = line(self.top + rows).unwrap_or_else(|| self.buffer.rope().len_lines());
        (first, after)
    }

    pub fn top(&self) -> usize {
        self.top
    }

    /// Where the view is scrolled to, to put it back later.
    pub fn view(&self) -> (usize, usize) {
        (self.top, self.left)
    }

    pub fn set_view(&mut self, (top, left): (usize, usize)) {
        self.top = top;
        self.left = left;
    }

    /// Shows the primary cursor, in the middle of the pane if it was out of view, as Go to
    /// Symbol does in VS Code.
    pub fn reveal_centred(&mut self, area: Rect) {
        if !self.rendered {
            // `top` counts rows as shown, which folds and wrapping make other than lines.
            self.open_folds_at_carets();
            let text = self.text_area(area);
            let rows = text.h.max(1) as usize;
            let head = self.selections.primary().head;
            let (row, _) = self.place_of(&self.code_rows(text.w), head);
            if row < self.top || row >= self.top + rows {
                self.top = row.saturating_sub(rows / 2).min(self.last_top());
            }
        }
        self.reveal(area);
    }

    /// The lines or page rows there are to scroll through.
    fn scroll_length(&self) -> usize {
        // A cached page is close enough to bound the scroll; draw corrects any slack.
        let rows = self.page.borrow().as_ref().map(|page| page.rows.len());
        rows.filter(|_| self.rendered)
            .unwrap_or_else(|| self.shown_rows().len())
    }

    /// The furthest down the view goes. A page goes on past its end, as VS Code's editor
    /// does, until its last row is a third of the way down, so its last block can always be
    /// brought clear of the status bar; code goes a row past its end, so its last line never
    /// sits on the status bar.
    fn last_top(&self) -> usize {
        let rows = self.view_rows.get().max(1) as usize;
        let length = self.scroll_length();
        // A page that fits has nothing to clear.
        let past = if self.rendered && length > rows {
            rows - rows / 3
        } else {
            usize::from(!self.rendered)
        };
        (length + past).saturating_sub(rows)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.last_top() as isize;
        self.top = (self.top as isize + delta).clamp(0, last) as usize;
    }

    /// What moves when the wheel does: the text, without the minimap beside it or the left
    /// column, where the sheet's rounded corners are.
    pub fn scroll_region(&self, area: Rect) -> Rect {
        let text = self.text_area(area);
        // Pinned headers stay where they are while the rows under them move.
        let pinned = if self.rendered {
            0
        } else {
            self.sticky(&self.code_rows(text.w), text).len() as u16
        };
        Rect::new(
            text.x + 1,
            text.y + pinned,
            text.w.saturating_sub(1),
            text.h - pinned,
        )
    }

    /// Scrolls just enough to show the primary cursor.
    pub fn reveal(&mut self, area: Rect) {
        let area = self.text_area(area);
        if self.rendered {
            let (row, _) = rendered::locate(&self.page(area), self.selections.primary().head);
            let height = area.h.max(1) as usize;
            if row < self.top {
                self.top = row;
            } else if row >= self.top + height {
                self.top = row + 1 - height;
            }
            self.left = 0;
            return;
        }
        self.open_folds_at_carets();
        let head = self.selections.primary().head;
        let laid = self.code_rows(area.w);
        let (line, _) = self.place_of(&laid, head);
        let rows = area.h.max(1) as usize;
        if line < self.top {
            self.top = line;
        } else if line >= self.top + rows {
            self.top = line + 1 - rows;
        }
        if self.wrap {
            self.left = 0;
            return;
        }
        let column = self.column_of(head);
        let cols = area.w.saturating_sub(self.gutter_width(area.w)).max(1) as usize;
        if column < self.left {
            self.left = column;
        } else if column >= self.left + cols {
            self.left = column + 1 - cols;
        }
    }

    pub fn run(&mut self, command: Command, area: Rect) {
        if self.run_view(command, area) {
            return;
        }
        let page = area.h.max(2) as usize - 1;
        if self.rendered {
            let text_area = self.text_area(area);
            self.prepare_for_command(command, text_area);
            let handled = match command {
                Command::Move(direction, extend) => self.move_on_page(text_area, direction, extend),
                Command::DeleteBackward if self.tracking => {
                    self.tracked_delete(false);
                    true
                }
                Command::DeleteForward if self.tracking => {
                    self.tracked_delete(true);
                    true
                }
                Command::Newline if self.selection_in_table(text_area) => {
                    self.next_row(text_area);
                    true
                }
                Command::Outdent if self.selection_in_table(text_area) => {
                    let rows = self.page(text_area);
                    let start = self.selections.primary().start();
                    if let Some(cell) = rendered::previous_cell(&rows, start) {
                        self.select_cell(text_area, cell);
                    }
                    true
                }
                Command::Newline => {
                    self.enter_on_page();
                    true
                }
                Command::DeleteBackward => {
                    self.backspace_on_page();
                    true
                }
                Command::Indent => {
                    self.tab_on_page(text_area);
                    true
                }
                _ => false,
            };
            if handled {
                self.reveal(area);
                return;
            }
        }
        if let Command::Move(direction, extend) = command
            && self.move_by_rows(self.text_area(area), direction, extend)
        {
            self.reveal(area);
            return;
        }
        match command {
            Command::Move(direction, extend) => {
                let motion = match direction {
                    Direction::Left => Motion::Left,
                    Direction::Right => Motion::Right,
                    Direction::Up => Motion::Up,
                    Direction::Down => Motion::Down,
                    Direction::WordLeft => Motion::WordLeft,
                    Direction::WordRight => Motion::WordRight,
                    Direction::LineStart => Motion::LineStart,
                    Direction::LineColumnZero => Motion::LineColumnZero,
                    Direction::LineEnd => Motion::LineEnd,
                    Direction::Top => Motion::Top,
                    Direction::Bottom => Motion::Bottom,
                    Direction::PageUp => Motion::PageUp(page),
                    Direction::PageDown => Motion::PageDown(page),
                    Direction::NextWordStart => Motion::NextWordStart,
                    Direction::FindChar(c, forward, till) => Motion::FindChar(c, forward, till),
                };
                self.selections
                    .apply_motion(self.buffer.rope(), motion, extend);
            }
            Command::Newline => self.newline(),
            Command::Type(c) => self.type_char(c),
            Command::ToggleComment => self.toggle_comment(),
            Command::MoveLineUp => self.move_line(-1),
            Command::MoveLineDown => self.move_line(1),
            Command::DuplicateLine => self.duplicate_line(),
            Command::DeleteLine => self.delete_line(),
            Command::InsertLineBelow | Command::InsertLineAbove => {
                self.insert_line(command == Command::InsertLineAbove);
            }
            Command::SelectAllMatches => {
                // Next match until no new one comes.
                self.buffer.select_next_match(&mut self.selections);
                if !self.selections.primary().is_empty() {
                    loop {
                        let before = self.selections.len();
                        self.buffer.select_next_match(&mut self.selections);
                        if self.selections.len() == before {
                            break;
                        }
                    }
                }
            }
            Command::AddCursorAbove | Command::AddCursorBelow => {
                let rope = self.buffer.rope();
                let primary = self.selections.primary();
                let line = rope.char_to_line(primary.head);
                let column = primary.head - rope.line_to_char(line);
                let next = if command == Command::AddCursorAbove {
                    line.checked_sub(1)
                } else {
                    Some(line + 1).filter(|l| *l < rope.len_lines())
                };
                if let Some(next) = next {
                    let width = self.line_text(next).chars().count();
                    let at = rope.line_to_char(next) + column.min(width);
                    self.selections.add(Selection::cursor(at));
                }
            }
            Command::Indent => {
                let unit = self.indentation.unit();
                self.buffer.type_text(&mut self.selections, &unit);
            }
            Command::Outdent => self.outdent(),
            Command::DeleteWordLeft => self.delete_to(Motion::WordLeft),
            Command::DeleteWordRight => self.delete_to(Motion::WordRight),
            Command::DeleteToLineStart => self.delete_to(Motion::LineColumnZero),
            Command::DeleteToLineEnd => self.delete_to(Motion::LineEnd),
            Command::DeleteBackward => self.backspace(),
            Command::DeleteForward => self.buffer.delete_forward(&mut self.selections),
            Command::Undo => {
                self.buffer.undo(&mut self.selections);
            }
            Command::Redo => {
                self.buffer.redo(&mut self.selections);
            }
            Command::SelectAll => {
                let len = self.buffer.rope().len_chars();
                self.selections.set_single(Selection {
                    anchor: 0,
                    head: len,
                    goal: None,
                });
            }
            Command::SelectLines(count) => {
                let rope = self.buffer.rope();
                let line = rope.char_to_line(self.selections.primary().head);
                let last = rope.len_lines();
                self.selections.set_single(Selection {
                    anchor: rope.line_to_char(line),
                    head: rope.line_to_char((line + count).min(last)),
                    goal: None,
                });
            }
            Command::SelectNextMatch => self.buffer.select_next_match(&mut self.selections),
            _ => return,
        }
        self.reveal(area);
    }

    /// Commands about how code is laid out: wrapping and folding. Pages lay themselves out,
    /// so there they do nothing. True when the command was one of these.
    fn run_view(&mut self, command: Command, area: Rect) -> bool {
        if !matches!(
            command,
            Command::ToggleWordWrap
                | Command::Fold
                | Command::Unfold
                | Command::FoldAll
                | Command::UnfoldAll
        ) {
            return false;
        }
        if self.rendered {
            return true;
        }
        let text = self.text_area(area);
        // The line at the top stays there.
        let line = self.code_rows(text.w).get(self.top).map_or(0, |r| r.line);
        if command == Command::ToggleWordWrap {
            self.wrap = !self.wrap;
            self.left = 0;
        } else {
            self.run_fold(command);
        }
        self.top = self.code_rows(text.w).row_of(line, 0);
        self.reveal(area);
        true
    }

    /// Moves up and down by the rows on screen, over folds and through wrapped lines, and
    /// Home and End to a wrapped row's ends before the line's. False when each line is a
    /// row, where moving by line is the same thing.
    fn move_by_rows(&mut self, area: Rect, direction: Direction, extend: bool) -> bool {
        let rows = self.code_rows(area.w);
        if matches!(*rows, Rows::Lines(_)) {
            return false;
        }
        let page = area.h.max(2) as isize - 1;
        let by = match direction {
            Direction::Up => -1,
            Direction::Down => 1,
            Direction::PageUp => -page,
            Direction::PageDown => page,
            Direction::LineStart | Direction::LineEnd if self.wrap => 0,
            _ => return false,
        };
        let rope = self.buffer.rope();
        let last = rows.len() - 1;
        let mut moved = Vec::new();
        for s in self.selections.iter() {
            let (index, column) = self.place_of(&rows, s.head);
            let row = rows.get(index).expect("a row");
            let line_start = rope.line_to_char(row.line);
            let (head, goal) = if by == 0 {
                let start = line_start + row.start;
                let end = self.row_char_at(row, usize::MAX);
                let head = match direction {
                    Direction::LineStart if !row.first() && s.head > start => start,
                    Direction::LineEnd if row.end != usize::MAX && s.head < end => end,
                    _ => {
                        let mut one = Selections::single(*s);
                        let motion = if direction == Direction::LineStart {
                            Motion::LineStart
                        } else {
                            Motion::LineEnd
                        };
                        one.apply_motion(rope, motion, extend);
                        one.primary().head
                    }
                };
                (head, None)
            } else {
                let goal = s.goal.unwrap_or(column);
                let target = index as isize + by;
                let head = if target < 0 {
                    0
                } else if target as usize > last {
                    rope.len_chars()
                } else {
                    let row = rows.get(target as usize).expect("a row");
                    self.row_char_at(row, goal)
                };
                (head, Some(goal))
            };
            moved.push(Selection {
                anchor: if extend { s.anchor } else { head },
                head,
                goal,
            });
        }
        self.selections.replace_all(moved);
        true
    }

    /// Edits start from a visible place. Moves start wherever the caret is.
    fn prepare_for_command(&mut self, command: Command, area: Rect) {
        if matches!(
            command,
            Command::Newline | Command::DeleteBackward | Command::DeleteForward | Command::Indent
        ) {
            let primary = self.selections.primary();
            if primary.is_empty() {
                let at = rendered::snap(&self.page(area), primary.head);
                self.selections.set_single(Selection::cursor(at));
            }
        }
    }

    fn outdent(&mut self) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let start = rope.line_to_char(rope.char_to_line(head));
        let line = rope.line(rope.char_to_line(head));
        let spaces = if line.chars().next() == Some('\t') {
            1
        } else {
            let size = self.indentation.size;
            line.chars().take(size).take_while(|c| *c == ' ').count()
        };
        if spaces > 0 {
            self.buffer
                .replace_range(&mut self.selections, start, start + spaces, "");
        }
    }

    fn char_at(&self, at: usize) -> Option<char> {
        let rope = self.buffer.rope();
        (at < rope.len_chars()).then(|| rope.char(at))
    }

    /// Keeps the line's indent, adds a level after an opening bracket, and puts a closing
    /// bracket on a line of its own.
    fn newline(&mut self) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.line(rope.char_to_line(head));
        let indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let before = head.checked_sub(1).and_then(|at| self.char_at(at));
        let after = self.char_at(head);
        let opens = matches!(before, Some('{' | '(' | '['));
        let closes = matches!(
            (before, after),
            (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
        );
        let unit = if opens {
            self.indentation.unit()
        } else {
            String::new()
        };
        let inner = format!("{indent}{unit}");
        let text = if closes {
            format!("\n{inner}\n{indent}")
        } else {
            format!("\n{inner}")
        };
        self.buffer.type_text(&mut self.selections, &text);
        if closes {
            let at = head + 1 + inner.chars().count();
            self.selections.set_single(Selection::cursor(at));
        }
    }

    /// Types a character, closing brackets and quotes, wrapping a selection in them, and
    /// stepping over a closing one that's already there.
    pub fn type_char(&mut self, c: char) {
        if self.tracking && self.rendered {
            self.tracked_type(c.encode_utf8(&mut [0; 4]));
            return;
        }
        let primary = self.selections.primary();
        let close = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' | '`' | '\'' => Some(c),
            _ => None,
        };
        let pairs = !self.rendered && self.selections.len() == 1;
        let after = self.char_at(primary.head);
        let before = primary.head.checked_sub(1).and_then(|at| self.char_at(at));
        if pairs
            && primary.is_empty()
            && after == Some(c)
            && matches!(c, ')' | ']' | '}' | '"' | '`' | '\'')
        {
            self.selections
                .set_single(Selection::cursor(primary.head + 1));
            return;
        }
        let text = c.to_string();
        let Some(close) = close.filter(|_| pairs) else {
            self.buffer.type_text(&mut self.selections, &text);
            return;
        };
        if !primary.is_empty() {
            let (start, end) = (primary.start(), primary.end());
            let inner = self.buffer.rope().slice(start..end).to_string();
            let wrapped = format!("{c}{inner}{close}");
            self.buffer
                .replace_range(&mut self.selections, start, end, &wrapped);
            self.selections.set_single(Selection {
                anchor: start + 1,
                head: end + 1,
                goal: None,
            });
            return;
        }
        // Beside a word, a quote is an apostrophe or a lifetime, and a bracket wraps it later.
        let by_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if by_word(after) || (c == close && by_word(before)) {
            self.buffer.type_text(&mut self.selections, &text);
            return;
        }
        self.buffer
            .type_text(&mut self.selections, &format!("{c}{close}"));
        self.selections
            .set_single(Selection::cursor(primary.head + 1));
    }

    /// Deletes both halves of an empty pair.
    fn backspace(&mut self) {
        let primary = self.selections.primary();
        let pair = primary.head.checked_sub(1).and_then(|at| {
            let open = self.char_at(at)?;
            let close = self.char_at(primary.head)?;
            matches!(
                (open, close),
                ('(', ')') | ('[', ']') | ('{', '}') | ('"', '"') | ('`', '`') | ('\'', '\'')
            )
            .then_some(at)
        });
        match pair.filter(|_| primary.is_empty() && self.selections.len() == 1) {
            Some(at) => {
                self.buffer
                    .replace_range(&mut self.selections, at, at + 2, "");
                self.selections.set_single(Selection::cursor(at));
            }
            None => self.buffer.delete_backward(&mut self.selections),
        }
    }

    /// The whole lines the selection touches, as (first, last, start, end in chars).
    fn selected_lines(&self) -> (usize, usize, usize, usize) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        (first, last, start, end)
    }

    fn toggle_comment(&mut self) {
        let Some(mark) = self.buffer.path().and_then(Registry::line_comment) else {
            return;
        };
        let (first, last, start, end) = self.selected_lines();
        let lines: Vec<String> = (first..=last).map(|line| self.line_text(line)).collect();
        let written = |line: &&String| !line.trim().is_empty();
        let commented = lines
            .iter()
            .filter(written)
            .all(|line| line.trim_start().starts_with(mark));
        let margin = lines
            .iter()
            .filter(written)
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);
        let changed: Vec<String> = lines
            .iter()
            .map(|line| {
                if line.trim().is_empty() {
                    line.clone()
                } else if commented {
                    let at = line.len() - line.trim_start().len();
                    let rest = line[at + mark.len()..]
                        .strip_prefix(' ')
                        .unwrap_or(&line[at + mark.len()..]);
                    format!("{}{rest}", &line[..at])
                } else {
                    format!("{}{mark} {}", &line[..margin], &line[margin..])
                }
            })
            .collect();
        let text = changed.join("\n");
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        self.selections
            .set_single(Selection::cursor(start + text.chars().count()));
    }

    fn move_line(&mut self, by: isize) {
        let (first, last, start, end) = self.selected_lines();
        let rope = self.buffer.rope();
        let count = rope.len_lines();
        let trailing = rope.line(count - 1).len_chars() == 0;
        let lines = count - usize::from(trailing);
        let other = if by < 0 {
            first.checked_sub(1)
        } else {
            (last + 1 < lines).then_some(last + 1)
        };
        let Some(other) = other else {
            return;
        };
        let head = self.selections.primary().head;
        let block = rope.slice(start..end).to_string();
        let neighbour = self.line_text(other);
        let (from, to, text, moved) = if by < 0 {
            let from = rope.line_to_char(other);
            (
                from,
                end,
                format!("{block}\n{neighbour}"),
                from + head - start,
            )
        } else {
            let to = rope.line_to_char(other) + neighbour.chars().count();
            let shift = neighbour.chars().count() + 1;
            (start, to, format!("{neighbour}\n{block}"), head + shift)
        };
        self.buffer
            .replace_range(&mut self.selections, from, to, &text);
        self.selections.set_single(Selection::cursor(moved));
    }

    /// Takes out the lines the selection touches, newline and all.
    fn delete_line(&mut self) {
        let (first, last, start, _) = self.selected_lines();
        let rope = self.buffer.rope();
        let lines = rope.len_lines();
        let end = if last + 1 < lines {
            rope.line_to_char(last + 1)
        } else {
            rope.len_chars()
        };
        // The last line takes the newline before it instead.
        let start = if end == rope.len_chars() && first > 0 && start > 0 {
            start - 1
        } else {
            start
        };
        self.buffer
            .replace_range(&mut self.selections, start, end, "");
        let rope = self.buffer.rope();
        let line = first.min(rope.len_lines().saturating_sub(1));
        self.selections
            .set_single(Selection::cursor(rope.line_to_char(line)));
    }

    /// Opens a line below this one, or above, at its indent, and goes there.
    fn insert_line(&mut self, above: bool) {
        let (first, last, start, end) = self.selected_lines();
        let line = if above { first } else { last };
        let indent: String = self
            .line_text(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let (at, text) = if above {
            (start, format!("{indent}\n"))
        } else {
            (end, format!("\n{indent}"))
        };
        self.buffer
            .replace_range(&mut self.selections, at, at, &text);
        let caret = if above {
            start + indent.chars().count()
        } else {
            at + text.chars().count()
        };
        self.selections.set_single(Selection::cursor(caret));
    }

    fn duplicate_line(&mut self) {
        let (_, _, start, end) = self.selected_lines();
        let block = self.buffer.rope().slice(start..end).to_string();
        let head = self.selections.primary().head;
        let copy = format!("\n{block}");
        self.buffer
            .replace_range(&mut self.selections, end, end, &copy);
        let at = head + block.chars().count() + 1;
        self.selections.set_single(Selection::cursor(at));
    }

    pub fn go_to_line(&mut self, line: usize, area: Rect) {
        let at = self.buffer.position(line.saturating_sub(1), 0);
        self.selections.set_single(Selection::cursor(at));
        self.reveal(area);
    }

    /// Where the caret is on screen, if it's in view.
    pub fn caret_cell(&self, area: Rect) -> Option<(f32, u16)> {
        self.cell_of(area, self.selections.primary().head)
    }

    /// Where a place in the text is on screen, if it's in view.
    pub fn cell_of(&self, area: Rect, head: usize) -> Option<(f32, u16)> {
        let area = self.text_area(area);
        let (row, x) = if self.rendered {
            let rows = self.page(area);
            let (row, x) = rendered::locate(&rows, head);
            (row, (area.x + Self::page_margin(area)) as f32 + x)
        } else {
            let (row, column) = self.place_of(&self.code_rows(area.w), head);
            let column = column.checked_sub(self.left())?;
            let gutter = self.gutter_width(area.w);
            (row, (area.x + gutter) as f32 + column as f32)
        };
        let y = row.checked_sub(self.top)?;
        (y < area.h as usize).then_some((x, area.y + y as u16))
    }

    /// Takes a finished highlight. Returns whether there's something new to draw.
    pub fn collect_highlights(&self) -> bool {
        let Some((revision, spans)) = self.worker.get().and_then(Worker::finished) else {
            return false;
        };
        // It was asked for at this revision's size, or the next answer will correct it.
        if revision == self.buffer.revision() {
            self.spans_bytes.set(self.buffer.rope().len_bytes());
            *self.spans.borrow_mut() = Some((revision, spans));
            // Its bars were drawn without colours.
            *self.minimap.borrow_mut() = None;
            true
        } else {
            false
        }
    }

    /// Deletes from the cursor to where a motion lands, or just the selection if any.
    fn delete_to(&mut self, motion: Motion) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            let mut moved = self.selections.clone();
            moved.apply_motion(self.buffer.rope(), motion, false);
            let target = moved.primary().head;
            (target.min(primary.head), target.max(primary.head))
        } else {
            (primary.start(), primary.end())
        };
        if self.tracking && self.rendered {
            self.selections.set_single(Selection {
                anchor: start,
                head: end,
                goal: None,
            });
            self.tracked_delete(false);
            return;
        }
        self.buffer
            .replace_range(&mut self.selections, start, end, "");
        self.selections.set_single(Selection::cursor(start));
    }

    fn toggle_inline(&mut self, marker: &str) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            self.buffer.word_at(primary.head)
        } else {
            (primary.start(), primary.end())
        };
        let size = marker.chars().count();
        let rope = self.buffer.rope();
        let before = start
            .checked_sub(size)
            .filter(|from| rope.slice(*from..start) == marker);
        let after_end = end + size;
        let after = after_end <= rope.len_chars() && rope.slice(end..after_end) == marker;
        let inner = rope.slice(start..end).to_string();
        let (from, to, text, selected) = match before.filter(|_| after) {
            Some(from) => (from, after_end, inner, from),
            None => (start, end, format!("{marker}{inner}{marker}"), start + size),
        };
        let length = end - start;
        self.buffer
            .replace_range(&mut self.selections, from, to, &text);
        self.selections.set_single(Selection {
            anchor: selected,
            head: selected + length,
            goal: None,
        });
    }

    /// The prefix a selected line should get, given the one it has.
    fn next_prefix(format: Format, line: &str) -> (usize, String) {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let heading = hashes > 0 && line[hashes..].starts_with(' ');
        let simple = |prefix: &str| {
            if line.starts_with(prefix) {
                (prefix.len(), String::new())
            } else {
                (0, prefix.to_string())
            }
        };
        match format {
            Format::Heading if heading && hashes < MAX_HEADING_CYCLE => {
                (hashes + 1, format!("{} ", "#".repeat(hashes + 1)))
            }
            Format::Heading if heading => (hashes + 1, String::new()),
            Format::Heading => (0, "# ".to_string()),
            Format::Level(level) => {
                let drop = if heading { hashes + 1 } else { 0 };
                let add = "#".repeat(level as usize);
                (drop, if level == 0 { add } else { format!("{add} ") })
            }
            Format::Numbered => simple("1. "),
            Format::Bullet => simple("- "),
            Format::Task => simple("- [ ] "),
            Format::PlainText if heading => (hashes + 1, String::new()),
            Format::PlainText => {
                let markers = ["- [ ] ", "- [x] ", "- ", "* ", "> "];
                let found = markers.iter().find(|m| line.starts_with(**m));
                (found.map_or(0, |m| m.len()), String::new())
            }
            _ => simple("> "),
        }
    }

    fn toggle_lines(&mut self, format: Format) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        let mut text = Vec::new();
        for line in first..=last {
            let line = self.line_text(line);
            let (drop, add) = Self::next_prefix(format, &line);
            text.push(format!("{add}{}", &line[drop..]));
        }
        let text = text.join("\n");
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        let at = start + text.chars().count();
        self.selections.set_single(Selection::cursor(at));
    }

    /// Wraps the word or selection in a command, or takes it back out.
    fn toggle_command(&mut self, open: &str) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            self.buffer.word_at(primary.head)
        } else {
            (primary.start(), primary.end())
        };
        let size = open.chars().count();
        let rope = self.buffer.rope();
        let inside = start >= size
            && rope.slice(start - size..start) == open
            && rope.get_char(end) == Some('}');
        let inner = rope.slice(start..end).to_string();
        let length = end - start;
        let selected = if inside {
            let (from, to) = (start - size, end + 1);
            self.buffer
                .replace_range(&mut self.selections, from, to, &inner);
            from
        } else {
            let text = format!("{open}{inner}}}");
            self.buffer
                .replace_range(&mut self.selections, start, end, &text);
            start + size
        };
        self.selections.set_single(Selection {
            anchor: selected,
            head: selected + length,
            goal: None,
        });
    }

    /// The caret's line and its text, with any section command taken off.
    fn line_as_text(&self) -> (usize, String) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        let text = self.line_text(line);
        let plain = text
            .split_once('{')
            .filter(|_| text.trim_start().starts_with("\\"))
            .and_then(|(_, rest)| rest.trim_end().strip_suffix('}'))
            .unwrap_or(&text)
            .to_string();
        (line, plain)
    }

    fn set_section(&mut self, level: u8) {
        let (line, plain) = self.line_as_text();
        let start = self.buffer.rope().line_to_char(line);
        let end = start + self.line_text(line).chars().count();
        let text = match level {
            1 => format!("\\section{{{plain}}}"),
            2 => format!("\\subsection{{{plain}}}"),
            3 => format!("\\subsubsection{{{plain}}}"),
            _ => plain,
        };
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        let at = start + text.chars().count();
        self.selections.set_single(Selection::cursor(at));
    }

    /// Puts the selected lines in an environment, each one an `\\item` if it's a list.
    fn wrap_environment(&mut self, name: &str, items: bool) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        let mut text = format!("\\begin{{{name}}}\n");
        for line in first..=last {
            let lead = if items { "  \\item " } else { "  " };
            text.push_str(&format!("{lead}{}\n", self.line_text(line).trim()));
        }
        text.push_str(&format!("\\end{{{name}}}"));
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        let at = start + text.chars().count() - name.len() - 7;
        self.selections.set_single(Selection::cursor(at));
    }

    fn format_latex(&mut self, format: Format) {
        match format {
            Format::Bold => self.toggle_command("\\textbf{"),
            Format::Italic => self.toggle_command("\\emph{"),
            Format::Code => self.toggle_command("\\texttt{"),
            Format::Strikethrough => {
                // `\\sout` is ulem's, which a paper needs to build.
                self.ensure_package("[normalem]{ulem}", "{ulem}");
                self.toggle_command("\\sout{");
            }
            Format::Link => self.make_link("https://"),
            Format::Bullet | Format::Task => self.wrap_environment("itemize", true),
            Format::Numbered => self.wrap_environment("enumerate", true),
            Format::Quote => self.wrap_environment("quote", false),
            Format::CodeBlock => self.wrap_environment("verbatim", false),
            Format::Level(level) => self.set_section(level),
            Format::Heading => self.set_section(1),
            Format::PlainText => self.set_section(0),
            Format::Rule => self.insert_block("\\bigskip\\hrule\\bigskip", 0),
            Format::Table => self.insert_table(3, 3),
        }
    }

    pub fn format(&mut self, format: Format, area: Rect) {
        self.prepare_edit(area);
        if self.dialect() == Dialect::Latex {
            self.format_latex(format);
            return;
        }
        match format {
            Format::Bold => self.toggle_inline("**"),
            Format::Italic => self.toggle_inline("*"),
            Format::Code => self.toggle_inline("`"),
            Format::Strikethrough => self.toggle_inline("~~"),
            Format::CodeBlock => self.fence_lines(),
            Format::Table => self.insert_table(3, 3),
            Format::Rule => self.insert_block("---", 3),
            Format::Link => self.make_link("https://"),
            _ => self.toggle_lines(format),
        }
    }

    /// Makes the word or selection a link to `address`.
    pub fn make_link(&mut self, address: &str) {
        let primary = self.selections.primary();
        let (start, end) = if primary.is_empty() {
            self.buffer.word_at(primary.head)
        } else {
            (primary.start(), primary.end())
        };
        let text = self.buffer.rope().slice(start..end).to_string();
        let link = if self.dialect() == Dialect::Latex {
            format!("\\href{{{address}}}{{{text}}}")
        } else {
            format!("[{text}]({address})")
        };
        self.buffer
            .replace_range(&mut self.selections, start, end, &link);
        let after = start + link.chars().count();
        self.selections.set_single(Selection::cursor(after));
    }

    /// Finds where each comment's quote is, for tinting it.
    pub fn set_quotes(&self, quotes: &[(usize, String)]) {
        let rope = self.buffer.rope();
        let mut ranges = Vec::new();
        for (line, quote) in quotes {
            let index = line.saturating_sub(1);
            if quote.is_empty() || index >= rope.len_lines() {
                continue;
            }
            let text = self.line_text(index);
            if let Some(at) = text.find(quote.as_str()) {
                let start = rope.line_to_char(index) + text[..at].chars().count();
                ranges.push((start, start + quote.chars().count()));
            }
        }
        *self.quoted.borrow_mut() = ranges;
    }

    /// Puts text in a block of its own below this line, with the caret `caret` chars in.
    pub fn insert_block(&mut self, block: &str, caret: usize) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.char_to_line(head);
        let text = self.line_text(line);
        let at = rope.line_to_char(line) + text.chars().count();
        let gap = if text.trim().is_empty() { "" } else { "\n\n" };
        let insert = format!("{gap}{block}\n");
        self.buffer
            .replace_range(&mut self.selections, at, at, &insert);
        let spot = at + gap.chars().count() + caret;
        self.selections.set_single(Selection::cursor(spot));
    }

    /// Adds an image: a figure with a caption in a paper, a plain image in a note.
    pub fn insert_image(&mut self, path: &str) {
        if self.dialect() == Dialect::Latex {
            let figure = format!(
                "\\begin{{figure}}[h]\n  \\centering\n  \\includegraphics[width=\\linewidth]{{{path}}}\n  \\caption{{}}\n\\end{{figure}}"
            );
            let caret = figure.find("\\caption{}").unwrap_or(0) + "\\caption{".len();
            self.insert_block(&figure, caret);
        } else {
            self.insert_block(&format!("![]({path})"), 2);
        }
    }

    /// Writes this file, through its notebook if it has one. The project's `.editorconfig`
    /// tidies the lines first, as an edit that can be undone.
    pub fn save(&mut self) -> io::Result<()> {
        if self.notebook.is_none() {
            let edits = self.rules.edits(&self.buffer.text());
            self.buffer.edit_ranges(&mut self.selections, &edits);
        }
        self.save_as_is()
    }

    /// Writes this file as it stands, as saving in a pause while typing does: tidying then
    /// would take away the space just typed.
    pub fn save_as_is(&mut self) -> io::Result<()> {
        match &self.notebook {
            Some(notebook) => {
                let written = notebook.write(&self.buffer.text());
                self.buffer.save_as(&written)
            }
            None => self.buffer.save(),
        }
    }

    /// Takes the file back from disk, through its notebook if it has one.
    pub fn reload(&mut self) -> io::Result<()> {
        let notebook = self.notebook.is_some();
        let read = self
            .buffer
            .path()
            .filter(|_| notebook)
            .map(std::fs::read_to_string);
        let Some(text) = read else {
            return self.buffer.reload(&mut self.selections);
        };
        let notebook = Notebook::read(&text?).map_err(io::Error::other)?;
        let page = notebook.page();
        self.notebook = Some(notebook);
        *self.page.borrow_mut() = None;
        self.buffer.reload_as(&mut self.selections, page)
    }

    /// The cell a source line is in, as its marker's line, what it says of itself and its
    /// code. None for a line that isn't in a code cell.
    pub fn code_cell_at(&self, line: usize) -> Option<(usize, Marker, String)> {
        self.notebook.as_ref()?;
        let last = self.buffer.rope().len_lines();
        let top = (0..=line.min(last.saturating_sub(1)))
            .rev()
            .find(|n| marker(&self.line_text(*n)).is_some())?;
        let mark = marker(&self.line_text(top)).filter(|m| m.code)?;
        let lines = (top + 1..last).map(|n| self.line_text(n));
        let lines: Vec<String> = lines.take_while(|text| marker(text).is_none()).collect();
        let open = lines.iter().position(|l| l.starts_with("```"))?;
        let close = lines.iter().rposition(|l| l.trim() == "```")?;
        (close > open).then(|| (top, mark, lines[open + 1..close].join("\n")))
    }

    /// Every code cell, top to bottom, as its number and code.
    pub fn code_cells(&self) -> Vec<(Marker, String)> {
        let lines = 0..self.buffer.rope().len_lines();
        let tops = lines.filter(|n| marker(&self.line_text(*n)).is_some_and(|m| m.code));
        let cells = tops.filter_map(|n| self.code_cell_at(n));
        cells.map(|(_, mark, code)| (mark, code)).collect()
    }

    /// Writes new cells into the file, so they have numbers of their own to run as.
    pub fn number_new_cells(&mut self) -> io::Result<()> {
        let head = self.selections.primary().head;
        self.save()?;
        self.reload()?;
        self.selections.set_single(Selection::cursor(head));
        Ok(())
    }

    /// Adds a cell after the one the caret is in, with the caret ready in it.
    pub fn insert_cell(&mut self, code: bool) {
        let language = self
            .notebook
            .as_ref()
            .map_or("python", |n| n.language.as_str());
        let cell = if code {
            cellix_doc::notebook::new_code_cell(language)
        } else {
            cellix_doc::notebook::new_text_cell()
        };
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        let next = (line + 1..rope.len_lines()).find(|n| marker(&self.line_text(*n)).is_some());
        let at = next.map_or(rope.len_chars(), |n| rope.line_to_char(n));
        let caret = if code {
            cell.find("\n\n").map_or(0, |gap| gap + 1)
        } else {
            cell.chars().count()
        };
        let lead = if at == rope.len_chars() && !self.buffer.text().ends_with("\n\n") {
            "\n"
        } else {
            ""
        };
        let block = format!("{lead}{cell}\n\n");
        self.buffer
            .replace_range(&mut self.selections, at, at, &block);
        let spot = at + lead.chars().count() + cell[..caret].chars().count();
        self.selections.set_single(Selection::cursor(spot));
    }

    /// Moves to the first line of the next code cell, if there is one.
    pub fn go_to_next_cell(&mut self, area: Rect) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(self.selections.primary().head);
        let next = (line + 1..rope.len_lines())
            .find(|n| marker(&self.line_text(*n)).is_some_and(|m| m.code));
        if let Some(next) = next {
            self.go_to_line(next + 3, area);
        }
    }

    /// What's wrong with this revision of the text.
    fn current_issues(&self) -> &[Issue] {
        if self.issues.0 == self.buffer.revision() {
            &self.issues.1
        } else {
            &[]
        }
    }

    /// What this document is written in, for the prose checker.
    pub fn prose_source(&self) -> Source {
        match self.dialect() {
            Dialect::Latex => Source::Latex,
            _ => Source::Markdown,
        }
    }

    /// The spelling or grammar issue at the caret.
    pub fn issue_at_caret(&self) -> Option<&Issue> {
        let head = self.selections.primary().head;
        let holds = |issue: &&Issue| issue.start <= head && head <= issue.end;
        self.current_issues().iter().find(holds)
    }

    /// The words of a range of the document.
    pub fn words_in(&self, start: usize, end: usize) -> String {
        self.buffer.rope().slice(start..end).to_string()
    }

    /// Puts a suggested word in place, and forgets the issue it settles.
    pub fn apply_fix(&mut self, start: usize, end: usize, text: &str) {
        self.buffer
            .replace_range(&mut self.selections, start, end, text);
        let at = start + text.chars().count();
        self.selections.set_single(Selection::cursor(at));
    }

    /// Takes a word off the list of what's wrong, wherever it stands.
    pub fn forget_issues(&mut self, word: &str) {
        let wanted = word.to_lowercase();
        let rope = self.buffer.rope().clone();
        self.issues.1.retain(|issue| {
            rope.slice(issue.start..issue.end)
                .to_string()
                .to_lowercase()
                != wanted
        });
    }

    fn syntax(&self) -> Syntax {
        match self.dialect() {
            Dialect::Latex => Syntax::Latex,
            _ => Syntax::Critic,
        }
    }

    fn changes(&self) -> Vec<Change> {
        changes::find(&self.buffer.text(), self.syntax())
    }

    /// The change the caret is in or right after.
    pub fn change_at_caret(&self) -> Option<Change> {
        let head = self.selections.primary().head;
        let holds = |c: &Change| c.start <= head && head <= c.end;
        self.changes().into_iter().find(holds)
    }

    /// Keeps or drops the change at the caret.
    pub fn settle_change(&mut self, accept: bool) {
        let Some(change) = self.change_at_caret() else {
            return;
        };
        let kept = if accept { &change.new } else { &change.old };
        self.buffer
            .replace_range(&mut self.selections, change.start, change.end, kept);
        let at = change.start + kept.chars().count();
        self.selections.set_single(Selection::cursor(at));
    }

    /// Keeps or drops every change, and says how many there were.
    pub fn settle_all_changes(&mut self, accept: bool) -> usize {
        let text = self.buffer.text();
        let count = changes::find(&text, self.syntax()).len();
        if count > 0 {
            let settled = changes::settle_all(&text, self.syntax(), accept);
            let total = self.buffer.rope().len_chars();
            self.buffer
                .replace_range(&mut self.selections, 0, total, &settled);
            let head = self.selections.primary().head.min(settled.chars().count());
            self.selections.set_single(Selection::cursor(head));
        }
        count
    }

    /// Adds `\\usepackage<what>` before `\\begin{document}`, unless `known` is already used.
    fn ensure_package(&mut self, what: &str, known: &str) {
        let text = self.buffer.text();
        if text.contains(known) {
            return;
        }
        let Some(at) = text.find("\\begin{document}") else {
            return;
        };
        let at = text[..at].chars().count();
        let mut selections = self.selections.clone();
        let line = format!("\\usepackage{what}\n");
        self.buffer.replace_range(&mut selections, at, at, &line);
        let length = line.chars().count();
        // The caret stays on the words it was on.
        let shifted: Vec<Selection> = self
            .selections
            .iter()
            .map(|s| Selection {
                anchor: s.anchor + length,
                head: s.head + length,
                goal: s.goal,
            })
            .collect();
        if let Some(first) = shifted.first() {
            self.selections.set_single(*first);
            for s in &shifted[1..] {
                self.selections.add(*s);
            }
        }
    }

    /// Turns tracking on or off. A paper needs the package that prints its marks.
    pub fn toggle_tracking(&mut self) -> bool {
        self.tracking = !self.tracking;
        let text = self.buffer.text();
        if self.tracking
            && self.dialect() == Dialect::Latex
            && !text.contains("{changes}")
            && let Some(at) = text.find("\\begin{document}")
        {
            let at = text[..at].chars().count();
            let mut selections = self.selections.clone();
            self.buffer
                .replace_range(&mut selections, at, at, "\\usepackage{changes}\n");
        }
        self.tracking
    }

    /// Where typing goes: a caret inside something crossed out stands after it.
    fn tracked_head(&self, changes: &[Change]) -> usize {
        let head = self.selections.primary().head;
        let deleted = changes
            .iter()
            .find(|c| c.kind == Kind::Delete && c.start < head && head < c.end);
        deleted.map_or(head, |c| c.end)
    }

    /// Takes out what's selected, as a suggestion if changes are being tracked.
    pub fn cut_selection(&mut self) {
        if self.tracking && self.rendered {
            self.tracked_delete(false);
        } else {
            self.buffer.replace(&mut self.selections, "");
        }
    }

    /// Pastes as a suggestion.
    pub fn tracked_paste(&mut self, text: &str) {
        self.tracked_type(text);
    }

    /// Adds text as a suggestion, joining the one the caret is at the end of.
    fn tracked_type(&mut self, text: &str) {
        if !self.selections.primary().is_empty() {
            self.tracked_delete(false);
            let changes = self.changes();
            let head = self.selections.primary().head;
            if let Some(change) = changes.iter().find(|c| c.start == head) {
                self.selections.set_single(Selection::cursor(change.end));
            }
        }
        let changes = self.changes();
        let head = self.tracked_head(&changes);
        let (open, close) = self.syntax().insertion();
        let closing = close.chars().count();
        let joins = changes
            .iter()
            .find(|c| c.kind == Kind::Insert && c.start < head && head <= c.end);
        let (at, typed) = match joins {
            Some(change) => (head.min(change.end - closing), text.to_string()),
            None => (head, format!("{open}{text}{close}")),
        };
        self.buffer
            .replace_range(&mut self.selections, at, at, &typed);
        let caret = match joins {
            Some(_) => at + text.chars().count(),
            None => at + open.chars().count() + text.chars().count(),
        };
        self.selections.set_single(Selection::cursor(caret));
    }

    /// Crosses out what's selected or next to the caret. Words the suggestion added simply
    /// go, since nobody has seen them yet.
    fn tracked_delete(&mut self, forward: bool) {
        let primary = self.selections.primary();
        let changes = self.changes();
        let (open, close) = self.syntax().deletion();
        if !primary.is_empty() {
            let text = self
                .buffer
                .rope()
                .slice(primary.start()..primary.end())
                .to_string();
            let lines: Vec<String> = text
                .split('\n')
                .map(|line| {
                    if line.is_empty() {
                        String::new()
                    } else {
                        format!("{open}{line}{close}")
                    }
                })
                .collect();
            let start = primary.start();
            self.buffer.replace_range(
                &mut self.selections,
                start,
                primary.end(),
                &lines.join("\n"),
            );
            self.selections.set_single(Selection::cursor(start));
            return;
        }
        let head = self.selections.primary().head;
        let total = self.buffer.rope().len_chars();
        // Step over what's already crossed out, as the page shows it as gone.
        let settled = changes.iter().find(|c| {
            c.kind == Kind::Delete
                && if forward {
                    c.start <= head && head < c.end
                } else {
                    c.start < head && head <= c.end
                }
        });
        let head = settled.map_or(head, |c| if forward { c.end } else { c.start });
        let at = if forward {
            (head < total).then_some(head)
        } else {
            head.checked_sub(1)
        };
        let Some(at) = at else {
            return;
        };
        let (added_open, added_close) = self.syntax().insertion();
        let added = changes.iter().find(|c| {
            let inner =
                (c.start + added_open.chars().count())..(c.end - added_close.chars().count());
            c.kind == Kind::Insert && (inner.contains(&at) || (!forward && head == c.end))
        });
        if let Some(change) = added {
            // The last of a suggestion takes its markers with it.
            if change.new.chars().count() <= 1 {
                self.buffer
                    .replace_range(&mut self.selections, change.start, change.end, "");
                self.selections.set_single(Selection::cursor(change.start));
                return;
            }
            let inner_end = change.end - added_close.chars().count();
            let at = if head == change.end {
                inner_end - 1
            } else {
                at
            };
            self.buffer
                .replace_range(&mut self.selections, at, at + 1, "");
            self.selections.set_single(Selection::cursor(at));
            return;
        }
        let Some(gone) = self.char_at(at) else {
            return;
        };
        if gone == '\n' {
            self.buffer
                .replace_range(&mut self.selections, at, at + 1, "");
            self.selections.set_single(Selection::cursor(at));
            return;
        }
        // Next to a crossing out, it joins that one.
        let opening = open.chars().count();
        if let (false, Some(change)) = (
            forward,
            changes
                .iter()
                .find(|c| c.kind == Kind::Delete && c.start == head),
        ) {
            let joined = format!("{open}{gone}");
            self.buffer
                .replace_range(&mut self.selections, at, change.start + opening, &joined);
            self.selections.set_single(Selection::cursor(at));
            return;
        }
        let crossed = format!("{open}{gone}{close}");
        self.buffer
            .replace_range(&mut self.selections, at, at + 1, &crossed);
        let caret = if forward {
            at + crossed.chars().count()
        } else {
            at
        };
        self.selections.set_single(Selection::cursor(caret));
    }

    /// The line that names the image the caret is on. A paper's figure counts as a whole.
    fn image_line_at_caret(&self) -> Option<usize> {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.char_to_line(head);
        if self.dialect() == Dialect::Markdown {
            return cellix_doc::figure::image_line(&self.line_text(line)).map(|_| line);
        }
        let names = |n: &usize| self.line_text(*n).contains("\\includegraphics");
        if names(&line) {
            return Some(line);
        }
        let opens = |n: &usize| self.line_text(*n).contains("\\begin{figure");
        let closes = |n: &usize| self.line_text(*n).contains("\\end{figure");
        let first = (0..=line).rev().find(opens)?;
        let last = (line..rope.len_lines()).find(closes)?;
        (first..=last).find(names)
    }

    pub fn image_at_caret(&self) -> bool {
        self.image_line_at_caret().is_some()
    }

    /// Puts a new line in place of the image's, leaving the caret where the line starts so
    /// the picture shows again.
    fn rewrite_image(&mut self, line: usize, text: &str) {
        let start = self.buffer.rope().line_to_char(line);
        let end = start + self.line_text(line).chars().count();
        self.buffer
            .replace_range(&mut self.selections, start, end, text);
        self.selections.set_single(Selection::cursor(start));
    }

    /// Sets how much of the page the image at the caret takes, in percent.
    pub fn set_image_width(&mut self, percent: u32) {
        let Some(line) = self.image_line_at_caret() else {
            return;
        };
        let text = self.line_text(line);
        let sized = if self.dialect() == Dialect::Latex {
            cellix_doc::figure::set_graphics_width(&text, percent)
        } else {
            cellix_doc::figure::set_width(&text, percent)
        };
        if let Some(sized) = sized {
            self.rewrite_image(line, &sized);
        }
    }

    /// Shows another file in the image's place, keeping its words and size.
    pub fn replace_image(&mut self, path: &str) {
        let Some(line) = self.image_line_at_caret() else {
            return;
        };
        let text = self.line_text(line);
        let replaced = if self.dialect() == Dialect::Latex {
            cellix_doc::figure::set_graphics_path(&text, path)
        } else {
            cellix_doc::figure::set_path(&text, path)
        };
        if let Some(replaced) = replaced {
            self.rewrite_image(line, &replaced);
        }
    }

    /// Takes the image out, with the figure around it in a paper.
    pub fn remove_image(&mut self) {
        let Some(line) = self.image_line_at_caret() else {
            return;
        };
        let rope = self.buffer.rope();
        let (mut first, mut last) = (line, line);
        if self.dialect() == Dialect::Latex {
            let opens = |n: &usize| self.line_text(*n).contains("\\begin{figure");
            let closes = |n: &usize| self.line_text(*n).contains("\\end{figure");
            if let (Some(a), Some(b)) = (
                (0..=line).rev().find(opens),
                (line..rope.len_lines()).find(closes),
            ) {
                (first, last) = (a, b);
            }
        }
        let start = rope.line_to_char(first);
        let end = rope.line_to_char((last + 1).min(rope.len_lines() - 1));
        let end = end.max(start + self.line_text(last).chars().count());
        self.buffer
            .replace_range(&mut self.selections, start, end, "");
        self.selections.set_single(Selection::cursor(start));
    }

    /// Adds a diagram below this line, with the caret in its source to go on writing.
    pub fn insert_diagram(&mut self, language: &str, source: &str) {
        let block = format!("```{language}\n{source}\n```");
        self.insert_block(&block, language.chars().count() + 4);
    }

    /// Types text where the caret is.
    pub fn insert_text(&mut self, text: &str) {
        self.buffer.replace(&mut self.selections, text);
    }

    /// Whether the caret is inside a formula showing as source.
    pub fn in_formula(&self) -> bool {
        self.page
            .borrow()
            .as_ref()
            .is_some_and(|p| p.open.is_some())
    }

    /// Inserts a symbol or template. Inside a formula it goes in where the caret is, and
    /// anywhere else it starts a new formula.
    pub fn insert_math(&mut self, symbol: &Symbol) {
        let display = if self.dialect() == Dialect::Latex {
            ("\\begin{equation}\n", "\n\\end{equation}")
        } else {
            ("$$\n", "\n$$")
        };
        let (open, close, block) = match (self.in_formula(), symbol.block) {
            (true, _) => ("", "", false),
            (false, true) => (display.0, display.1, true),
            (false, false) => ("$", "$", false),
        };
        let chars = symbol.text.chars().count();
        let caret = symbol.caret.min(chars) + open.chars().count();
        // What's selected goes in the template's first slot, rather than being replaced.
        let selected = self.selected_text().unwrap_or_default();
        let slot = symbol.caret < chars && !selected.is_empty();
        let body = if slot {
            let at: String = symbol.text.chars().take(symbol.caret).collect();
            let rest: String = symbol.text.chars().skip(symbol.caret).collect();
            format!("{at}{selected}{rest}")
        } else {
            symbol.text.to_string()
        };
        let text = format!("{open}{body}{close}");
        if block {
            self.insert_block(&text, caret);
            return;
        }
        let primary = self.selections.primary();
        let (start, end) = (primary.start(), primary.end());
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        self.selections.set_single(Selection::cursor(start + caret));
    }

    /// The `tabular` the caret is in, read the same way as a Markdown table.
    fn tabular_at_caret(&self) -> Option<(usize, usize, usize, usize)> {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.char_to_line(head);
        let first = (0..=line)
            .rev()
            .find(|n| self.line_text(*n).contains("\\begin{tabular}"))?;
        let last =
            (line..rope.len_lines()).find(|n| self.line_text(*n).contains("\\end{tabular}"))?;
        let row = (first..line)
            .filter(|n| self.line_text(*n).contains("\\\\"))
            .count();
        let before: String = self
            .line_text(line)
            .chars()
            .take(head - rope.line_to_char(line))
            .collect();
        Some((first, last, row, before.matches('&').count()))
    }

    /// The table the caret is in, as its first and last lines and the caret's row and
    /// column, with the header as row zero and the rule left out.
    pub fn table_at_caret(&self) -> Option<(usize, usize, usize, usize)> {
        if self.dialect() == Dialect::Latex {
            return self.tabular_at_caret();
        }
        if let Dialect::Data(separator) = self.dialect() {
            let head = self.selections.primary().head;
            let rope = self.buffer.rope();
            let line = rope.char_to_line(head);
            let last = (0..rope.len_lines())
                .rev()
                .find(|n| !self.line_text(*n).is_empty())
                .unwrap_or(0);
            let before = rope.slice(rope.line_to_char(line)..head).to_string();
            return Some((
                0,
                last,
                line,
                cellix_doc::table::field_of(&before, separator),
            ));
        }
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = rope.char_to_line(head);
        let has_pipe = |n: usize| n < rope.len_lines() && self.line_text(n).contains('|');
        if !has_pipe(line) {
            return None;
        }
        let first = (0..=line)
            .rev()
            .take_while(|n| has_pipe(*n))
            .last()
            .unwrap_or(line);
        let last = (line..rope.len_lines())
            .take_while(|n| has_pipe(*n))
            .last()
            .unwrap_or(line);
        let rule = self.line_text(first + 1);
        let is_rule = |c: char| "|-: ".contains(c);
        if last == first || !rule.contains('-') || !rule.chars().all(is_rule) {
            return None;
        }
        let column = head - rope.line_to_char(line);
        let before: String = self.line_text(line).chars().take(column).collect();
        let pipes = before.matches('|').count().saturating_sub(1);
        let row = match line - first {
            0 => 0,
            1 => 1,
            below => below - 1,
        };
        Some((first, last, row, pipes))
    }

    /// Changes the table the caret is in, and leaves the caret in the cell that matters.
    pub fn edit_table(&mut self, edit: TableEdit) {
        let Some((first, last, row, column)) = self.table_at_caret() else {
            return;
        };
        let rope = self.buffer.rope();
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        let text = rope.slice(start..end).to_string();
        if let Dialect::Data(separator) = self.dialect() {
            if let Some((table, at)) =
                cellix_doc::table::edit_rows(&text, separator, row, column, edit)
            {
                self.buffer
                    .replace_range(&mut self.selections, start, end, &table);
                self.selections.set_single(Selection::cursor(start + at));
            }
            return;
        }
        if self.dialect() == Dialect::Latex {
            let table = cellix_doc::latex::edit_tabular(&text, row, column, edit);
            let table = table.unwrap_or_default();
            self.buffer
                .replace_range(&mut self.selections, start, end, &table);
            let row = row + usize::from(edit == TableEdit::RowBelow);
            let column = column + usize::from(edit == TableEdit::ColumnRight);
            let at = cellix_doc::latex::tabular_cell_offset(&table, row, column);
            self.selections.set_single(Selection::cursor(start + at));
            return;
        }
        match cellix_doc::table::apply(&text, row, column, edit) {
            Some((table, row, column)) => {
                self.buffer
                    .replace_range(&mut self.selections, start, end, &table);
                let at = start + cellix_doc::table::cell_offset(&table, row, column);
                self.selections.set_single(Selection::cursor(at));
            }
            None => {
                // Whatever's after the table closes up behind it.
                let total = self.buffer.rope().len_chars();
                self.buffer
                    .replace_range(&mut self.selections, start, (end + 1).min(total), "");
                self.selections.set_single(Selection::cursor(start));
            }
        }
    }

    /// What kind of paragraph the caret is in, and which formats it's already in.
    pub fn caret_formats(&self) -> (&'static str, Vec<Format>) {
        let head = self.selections.primary().head;
        let rope = self.buffer.rope();
        let line = self.line_text(rope.char_to_line(head));
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let trimmed = line.trim_start();
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        let mut active = Vec::new();
        if trimmed.starts_with("- [") {
            active.push(Format::Task);
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            active.push(Format::Bullet);
        } else if digits > 0 && trimmed[digits..].starts_with(". ") {
            active.push(Format::Numbered);
        } else if trimmed.starts_with("> ") {
            active.push(Format::Quote);
        }
        let cache = self.page.borrow();
        let rows = cache.as_ref().map(|p| p.rows.clone());
        let at = rows.iter().flat_map(|r| r.iter()).find_map(|row| {
            row.glyphs
                .iter()
                .find(|g| g.at == Some(head) || g.at.map(|a| a + 1) == Some(head))
        });
        if let Some(glyph) = at {
            if glyph.font.bold && hashes == 0 {
                active.push(Format::Bold);
            }
            if glyph.font.italic {
                active.push(Format::Italic);
            }
            if glyph.tone == Tone::Code {
                active.push(Format::Code);
            }
            if glyph.tone == Tone::Link {
                active.push(Format::Link);
            }
        }
        let style = match hashes {
            1 => "Heading 1",
            2 => "Heading 2",
            3 => "Heading 3",
            4 => "Heading 4",
            5 => "Heading 5",
            6 => "Heading 6",
            _ => "Text",
        };
        (style, active)
    }

    /// Puts the selected lines in a fenced block, with the caret on its first line.
    fn fence_lines(&mut self) {
        let primary = self.selections.primary();
        let rope = self.buffer.rope();
        let first = rope.char_to_line(primary.start());
        let last = rope.char_to_line(primary.end());
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + self.line_text(last).chars().count();
        let inner = rope.slice(start..end).to_string();
        let text = format!("```\n{inner}\n```");
        self.buffer
            .replace_range(&mut self.selections, start, end, &text);
        self.selections.set_single(Selection::cursor(start + 4));
    }

    /// Adds a new table below this line, with its first heading selected to type over.
    pub fn insert_table(&mut self, rows: usize, columns: usize) {
        if self.dialect() == Dialect::Latex {
            let table = cellix_doc::latex::new_table(rows, columns);
            let first = table.find("Column 1").unwrap_or(0);
            self.insert_block(&table, first);
            return;
        }
        let table = cellix_doc::table::new_table(rows, columns);
        self.insert_block(&table, 2);
        let head = self.selections.primary().head;
        self.selections.set_single(Selection {
            anchor: head,
            head: head + "Column 1".len(),
            goal: None,
        });
    }

    /// Selects the word at a place, as a double click does.
    pub fn select_word_at(&mut self, at: usize) {
        let (start, end) = self.buffer.word_at(at);
        self.selections.set_single(Selection {
            anchor: start,
            head: end,
            goal: None,
        });
    }

    /// Selects the whole line at a place, as a triple click does.
    pub fn select_line_at(&mut self, at: usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(at);
        let start = rope.line_to_char(line);
        let end = if line + 1 < rope.len_lines() {
            rope.line_to_char(line + 1)
        } else {
            rope.len_chars()
        };
        self.selections.set_single(Selection {
            anchor: start,
            head: end,
            goal: None,
        });
    }

    /// Whether this file has a page to show, whichever view it's in.
    pub fn has_page(&self) -> bool {
        self.buffer.path().is_some_and(|p| Dialect::of(p).is_some())
    }

    pub fn selected_text(&self) -> Option<String> {
        let rope = self.buffer.rope();
        let parts: Vec<String> = self
            .selections
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| rope.slice(s.start()..s.end()).to_string())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        theme: &Theme,
        focused: bool,
        registry: &Registry,
        marks: &Marks,
    ) {
        let full = area;
        let area = self.text_area(full);
        self.view_rows.set(area.h);
        self.view_width.set(area.w);
        let dark = !theme.is_light();
        if self.dark.replace(dark) != dark {
            *self.page.borrow_mut() = None;
            *self.library.borrow_mut() = Library::default();
        }
        if self.rendered {
            self.draw_page(grid, area, theme, focused, marks);
            if self.shows_minimap(full) {
                self.draw_minimap(grid, full, theme);
            } else {
                let track = Rect::new(area.right(), area.y, 1, area.h);
                self.draw_scrollbar(grid, track, self.page(area).len(), theme);
            }
            return;
        }
        let rows = self.code_rows(area.w);
        let track = Rect::new(area.right(), area.y, 1, area.h);
        if !self.shows_minimap(full) {
            self.draw_scrollbar(grid, track, rows.len(), theme);
        }
        grid.fill(area, theme.text);
        let gutter = self.gutter_width(area.w);
        let rope = self.buffer.rope();
        let current = rope.char_to_line(self.selections.primary().head);
        let left = self.left();
        let mut hint_rows = Vec::new();
        for offset in 0..area.h {
            let Some(laid) = rows.get(self.top + offset as usize) else {
                break;
            };
            let line = laid.line;
            let y = area.y + offset;
            let active = focused && line == current;
            let bg = if active { theme.line } else { theme.text.bg };
            let text_style = cellix_render::Style { bg, ..theme.text };
            grid.fill(Rect::new(area.x, y, area.w, 1), text_style);
            let number_fg = if marks.errors.contains(&(line + 1)) {
                theme.error
            } else if active {
                theme.number_active
            } else {
                theme.gutter.fg
            };
            let gutter_style = cellix_render::Style {
                fg: number_fg,
                bg,
                ..theme.gutter
            };
            if laid.first() {
                let number = format!("{:>w$}", line + 1, w = self.number_width(area.w));
                grid.put_str(area.x, y, &number, gutter_style, area.x + gutter);
                if Self::chevron_cells(area.w) > 0 && self.fold_at(line) == Some(true) {
                    let icon = Icon::ChevronRight;
                    self.draw_chevron(grid, (area.x + gutter, y), icon, (gutter_style, theme));
                }
            }
            if laid.first() && marks.comments.contains(&(line + 1)) {
                let mark = cellix_render::Style {
                    fg: theme.accent,
                    bg,
                    ..theme.gutter
                };
                grid.put_str(area.x, y, "◆", mark, area.x + gutter);
            }

            let chars: Vec<char> = self.line_text(line).chars().collect();
            let end = laid.end.min(chars.len());
            let from = laid.start.min(end);
            let mut column = if laid.first() {
                0
            } else {
                self.column_of(rope.line_to_char(line) + from)
            };
            let mut text = String::new();
            // Hints go in the row as it's shown, by where they start on screen and how wide.
            // One where a wrapped row ends shows at the start of the next.
            let origin = column;
            let mut hinted = 0;
            let start = rope.line_to_char(line);
            let mut shown_hints = Vec::new();
            let screen = |at: usize| laid.indent as isize + (at - origin) as isize - left as isize;
            for i in from..=end {
                if i < end || end == chars.len() {
                    for (_, label) in self.hints_in(start + i, start + i + 1) {
                        let at = column + hinted;
                        let cells = self.hint_cells(label).2;
                        // Its letters on cells; with pixels the chip is drawn over blank cells.
                        let shown = label.chars().chain(std::iter::repeat(' '));
                        let blank = std::iter::repeat_n(' ', cells);
                        let letters: Vec<char> = if self.measure.in_cells() {
                            shown.take(cells).collect()
                        } else {
                            blank.collect()
                        };
                        for (n, c) in letters.into_iter().enumerate() {
                            if at + n >= left {
                                text.push(c);
                            }
                        }
                        shown_hints.push((screen(at), label.as_str()));
                        hinted += cells;
                    }
                }
                let Some(c) = chars[..end].get(i) else {
                    break;
                };
                let width = display_width(*c, column, self.indentation.size);
                if column + hinted + width > left {
                    if *c == '\t' {
                        text.extend(std::iter::repeat_n(' ', width));
                    } else {
                        text.push(*c);
                    }
                }
                column += width;
            }
            let x = area.x + gutter + laid.indent;
            let after = grid.put_str(x, y, &text, text_style, area.right());
            hint_rows.push((y, bg, shown_hints));
            let last_row = rows
                .get(self.top + offset as usize + 1)
                .is_none_or(|r| r.line != line);
            if last_row && self.fold_at(line) == Some(true) {
                self.draw_fold_chip(grid, (after, y), area.right(), line, (text_style, theme));
            }
            let guide = cellix_render::Style {
                fg: theme.guide,
                bg,
                ..theme.text
            };
            let indent = if laid.first() {
                text.chars().take_while(|c| *c == ' ').count()
            } else {
                laid.indent as usize
            };
            for at in (0..indent).step_by(self.indentation.size) {
                let x = area.x + gutter + at as u16;
                if x < area.right() {
                    grid.put_str(x, y, "│", guide, area.right());
                }
            }
        }
        self.draw_highlights(grid, area, gutter, theme, registry);
        if self.bracket_pairs {
            self.draw_brackets(grid, area, gutter, theme, focused);
        }
        self.draw_hints(grid, area, gutter, &hint_rows, theme);
        if self.shows_minimap(full) {
            self.draw_minimap(grid, full, theme);
        }
        for (line, change) in &self.line_changes().1 {
            for index in rows.rows_of(*line) {
                if index >= self.top && index < self.top + area.h as usize {
                    let y = area.y + (index - self.top) as u16;
                    let x = area.x as f32 + gutter as f32 - 0.7;
                    self.draw_change(grid, x, y, *change, theme);
                }
            }
        }
        for (start, end) in self.found.iter().copied() {
            let s = Selection {
                anchor: start,
                head: end,
                goal: None,
            };
            self.draw_range(grid, area, (gutter, &rows), &s, theme.found);
        }
        for s in self.selections.iter().filter(|s| !s.is_empty()) {
            self.draw_range(grid, area, (gutter, &rows), s, theme.selection);
        }
        if focused {
            let head = self.selections.primary().head;
            let (row, column) = self.place_of(&rows, head);
            if row >= self.top && column >= left {
                let x = area.x as usize + gutter as usize + column - left;
                let y = area.y as usize + row - self.top;
                if x < area.right() as usize && y < area.bottom() as usize {
                    grid.cursor = Some((x as u16, y as u16));
                }
            }
        }
        self.draw_sticky(grid, area, (gutter, &rows), theme);
    }

    /// Each hint quiet on a chip of its own, as VS Code draws them: on the cell tier the
    /// cells take the chip's colour, and on the pixel tiers a rounded chip sits under smaller
    /// text. The space a hint asks for on either side stays outside the chip.
    #[allow(clippy::type_complexity)]
    fn draw_hints(
        &self,
        grid: &mut Grid,
        area: Rect,
        gutter: u16,
        rows: &[(u16, Rgb, Vec<(isize, &str)>)],
        theme: &Theme,
    ) {
        // Each hint is placed in cells from where the row's text starts, before any it's
        // scrolled past.
        let cells = self.measure.in_cells();
        let left = area.x + gutter;
        for (y, bg, hints) in rows {
            let (fg, chip) = theme.hint(*bg);
            for (at, label) in hints {
                let (lead, wide, total) = self.hint_cells(label);
                let body = label.trim();
                if body.is_empty() {
                    continue;
                }
                if cells {
                    let Ok(from) = u16::try_from(at + lead as isize) else {
                        continue;
                    };
                    let x = left + from;
                    if x >= area.right() {
                        continue;
                    }
                    let width = (wide as u16).min(area.right() - x);
                    grid.set_style(Rect::new(x, *y, width, 1), |style| cellix_render::Style {
                        fg,
                        bg: chip,
                        ..style
                    });
                    continue;
                }
                // With pixels its cells are blank and the chip is painted over them.
                let Ok(from) = u16::try_from(*at) else {
                    continue;
                };
                let x = left + from;
                if x >= area.right() {
                    continue;
                }
                let width = (total as u16).min(area.right() - x);
                grid.set_style(Rect::new(x, *y, width, 1), |style| cellix_render::Style {
                    fg,
                    bg: *bg,
                    painted: true,
                    ..style
                });
                let start = x as f32 + lead;
                let bounds = cellix_render::Bounds::cells(start, *y as f32 + 0.14, wide, 0.72);
                grid.panel(bounds, 8, Some(chip), None);
                // The text in the middle of its chip, smaller than the code.
                let font = Self::HINT_FONT;
                let room = self.measure.width(body, font);
                let at = start + (wide - room).max(0.0) / 2.0;
                let right = area.right() as f32;
                grid.text_run((at, *y as f32), right, body, (fg, false), font);
            }
        }
    }

    fn draw_scrollbar(&self, grid: &mut Grid, track: Rect, lines: usize, theme: &Theme) {
        grid.fill(track, theme.text);
        let rows = track.h as usize;
        if lines <= rows {
            return;
        }
        let height = (rows * rows / lines).max(1);
        // A page scrolls past its end, so the thumb stops at the bottom of its track rather
        // than run on over whatever stands under the file.
        let top = (self.top * rows / lines).min(rows.saturating_sub(height));
        // A slim rounded thumb on the pixel tiers, and a block of colour on cells.
        let cells = Rect::new(track.x, track.y + top as u16, 1, height as u16);
        let thumb = cellix_render::Style {
            fg: theme.scrollbar,
            painted: true,
            ..theme.text
        };
        for y in cells.y..cells.bottom() {
            grid.put_str(cells.x, y, "▐", thumb, cells.right());
        }
        let bounds =
            cellix_render::Bounds::cells(cells.x as f32 + 0.3, cells.y as f32, 0.5, cells.h as f32);
        grid.panel(bounds, 20, Some(theme.scrollbar), None);
    }

    /// Backgrounds of code blocks and tables, drawn before the text on top of them.
    fn draw_page_blocks(&self, grid: &mut Grid, area: Rect, rows: &[Row], theme: &Theme) {
        let left = (area.x + Self::page_margin(area)) as f32;
        let visible = self.top..(self.top + area.h as usize).min(rows.len());
        let y_of = |index: usize| area.y as f32 + (index - self.top) as f32;
        let mut index = visible.start;
        while index < visible.end {
            let row = &rows[index];
            let group =
                |r: &Row| (row.shaded && r.shaded) || (row.table.is_some() && r.table.is_some());
            if !group(row) {
                index += 1;
                continue;
            }
            let end = (index..visible.end)
                .find(|i| !group(&rows[*i]))
                .unwrap_or(visible.end);
            let (top, height) = (y_of(index), (end - index) as f32);
            if row.shaded {
                let width = area.right() as f32 - left - 1.0;
                let bounds = Bounds::cells(left, top - 0.08, width, height + 0.2);
                grid.panel(bounds, 22, Some(theme.code_bg), Some(theme.border));
            } else if let Some(table) = &row.table {
                // A table too wide even with its cells wrapped stops at the pane's edge
                // rather than run on over what's beside it.
                let room = area.right() as f32 - left;
                let width = table.columns.last().copied().unwrap_or(0.0).min(room);
                let bounds = Bounds::cells(left, top + 0.02, width, height - 0.04);
                let part_of = |at: usize| rows[at].table.as_ref().map(|t| (t.header, t.part));
                if table.header {
                    // Its line of the table can take several rows.
                    let rows = (index..end).take_while(|at| part_of(*at).is_some_and(|p| p.0));
                    let head = Bounds::cells(left, top + 0.02, width, rows.count() as f32);
                    grid.panel(head, 14, Some(theme.code_bg), None);
                }
                // A line under each line of the table, not under each row one takes.
                for at in (index + 1..end).filter(|at| part_of(*at).is_some_and(|p| p.1 == 0)) {
                    let line = Bounds::cells(left, y_of(at), width, 0.0);
                    grid.hairline(line, theme.popup_border);
                }
                let inner = table.columns.len().saturating_sub(2);
                for x in table.columns.iter().skip(1).take(inner) {
                    if *x >= width {
                        break;
                    }
                    let line = Bounds::cells(left + x, top + 0.02, 0.0, height - 0.04);
                    grid.hairline(line, theme.popup_border);
                }
                grid.panel(bounds, 14, None, Some(theme.popup_border));
            }
            index = end;
        }
    }

    fn draw_page(&self, grid: &mut Grid, area: Rect, theme: &Theme, focused: bool, marks: &Marks) {
        grid.fill(area, theme.text);
        let rows = self.page(area);
        let proportional = !self.measure.in_cells();
        let left = area.x + Self::page_margin(area);
        let primary = self.selections.primary();
        let (caret_row, caret_x) = rendered::locate(&rows, primary.head);
        if proportional {
            self.draw_page_blocks(grid, area, &rows, theme);
            self.draw_figures(grid, area, &rows, theme);
        }
        let changes = self.line_changes();
        for (index, row) in rows.iter().enumerate().skip(self.top) {
            let y = area.y + (index - self.top) as u16;
            if y >= area.bottom() {
                break;
            }
            let bg = if row.shaded && !proportional {
                theme.code_bg
            } else {
                theme.text.bg
            };
            if !proportional {
                grid.fill(
                    Rect::new(area.x, y, area.w, 1),
                    cellix_render::Style { bg, ..theme.text },
                );
            }
            // A tall heading's spacer shares its line, and takes no mark.
            let mut earlier = rows[..index]
                .iter()
                .rev()
                .take_while(|r| r.line == row.line);
            let first_row = row.stop && !earlier.any(|r| r.stop);
            if first_row && let Some(change) = changes.1.get(&row.line) {
                // Beside a tall heading's text, which sits in the row below its own.
                let mark = if row.tall { y + 1 } else { y };
                self.draw_change(grid, left as f32 - 1.4, mark, *change, theme);
            }
            if first_row && marks.comments.contains(&(row.line + 1)) {
                let mark = cellix_render::Style {
                    fg: theme.accent,
                    bg,
                    ..theme.text
                };
                grid.put_str(area.x + 1, y, "◆", mark, left);
            }
            // Xcode shows a heading's level in the margin, beside the caret.
            let commented = marks.comments.contains(&(row.line + 1));
            if proportional && focused && index == caret_row && row.heading > 0 && !commented {
                let dim = cellix_render::Style {
                    fg: theme.gutter.fg,
                    ..theme.text
                };
                self.draw_heading_mark(grid, row, (area.x, left, y), dim.fg);
            }
            if row.rule && proportional {
                let line = Bounds::cells(left as f32, y as f32 + 0.5, 60.0, 0.0);
                grid.hairline(line, theme.popup_border);
            }
            if row.quote {
                // Whole rows, so a block's bar runs on unbroken from row to row.
                let bar = Bounds::cells(left as f32 + 0.1, y as f32, 0.3, 1.0);
                let colour = row.callout.map(|kind| callout_colour(kind, theme));
                grid.panel(bar, 0, Some(colour.unwrap_or(theme.popup_border)), None);
            }
            // A tall heading on the last row would reach into the status bar, which paint
            // can't be clipped by, so it waits until it fits.
            if !(proportional && row.tall && y + 2 >= area.bottom()) {
                self.draw_page_row(grid, row, (left, y), area, bg, theme);
            }
        }
        if focused && primary.is_empty() && caret_row >= self.top {
            let y = area.y as usize + caret_row - self.top;
            let x = left as f32 + caret_x;
            self.draw_preview(grid, area, (x, y as f32), theme);
            if y < area.bottom() as usize && x < area.right() as f32 {
                let row = &rows[caret_row];
                let size = row.glyphs.first().map_or(100, |g| g.font.size) as f32 / 100.0;
                grid.set_caret(x, y as u16, (0.62 * size).min(1.6));
                if row.tall
                    && let Some(caret) = &mut grid.caret
                {
                    caret.y += units(TALL_DROP);
                }
            }
        }
    }

    /// Shows what the formula under the caret looks like, just below its line.
    fn draw_preview(&self, grid: &mut Grid, area: Rect, (x, y): (f32, f32), theme: &Theme) {
        let cache = self.page.borrow();
        let Some(page) = cache.as_ref() else {
            return;
        };
        let Some(figure) = page.preview.and_then(|index| page.figures.get(index)) else {
            return;
        };
        const PADDING: (f32, f32) = (1.0, 0.3);
        let (width, height) = (
            figure.width + PADDING.0 * 2.0,
            figure.height + PADDING.1 * 2.0,
        );
        let left = x.min(area.right() as f32 - width - 1.0).max(area.x as f32);
        let top = y + 1.2;
        let clip = Bounds::cells(area.x as f32, area.y as f32, area.w as f32, area.h as f32);
        grid.popup(
            Bounds::cells(left, top, width, height),
            22,
            theme.popup,
            theme.popup_border,
        );
        grid.paint.push(Paint::Image {
            bounds: Bounds::cells(
                left + PADDING.0,
                top + PADDING.1,
                figure.width,
                figure.height,
            ),
            clip,
            bitmap: figure.bitmap.clone(),
            color: theme.text.fg,
        });
    }

    /// The small `#` beside a heading, sitting on the heading's baseline.
    fn draw_heading_mark(
        &self,
        grid: &mut Grid,
        row: &Row,
        (x, left, y): (u16, u16, u16),
        color: Rgb,
    ) {
        /// How far a font's baseline sits below the middle of its line, in font sizes.
        const BASELINE: f32 = 0.35;
        const MARK_SIZE: u8 = 85;
        let size = row.glyphs.first().map_or(100, |g| g.font.size);
        let scale = (size as f32 - MARK_SIZE as f32) / 100.0;
        let drop = BASELINE * INTERFACE_SIZE * scale;
        let top = y as f32 + if row.tall { TALL_DROP } else { 0.0 } + drop;
        let font = Font {
            size: MARK_SIZE,
            ..Font::default()
        };
        grid.text_run(
            (x as f32 + 0.7, top),
            left as f32,
            "#",
            (color, false),
            font,
        );
    }

    fn draw_page_row(
        &self,
        grid: &mut Grid,
        row: &Row,
        (left, y): (u16, u16),
        area: Rect,
        bg: Rgb,
        theme: &Theme,
    ) {
        let proportional = !self.measure.in_cells();
        let origin = left as f32;
        let fg_of = |tone: Tone| match tone {
            Tone::Text | Tone::Block(None) => theme.text.fg,
            Tone::Heading => theme.heading,
            Tone::Code => theme.string,
            Tone::Block(Some(token)) => theme.token(token),
            Tone::Link | Tone::Check(_) => theme.accent,
            Tone::Dim | Tone::TableLine => theme.gutter.fg,
            Tone::Formula { .. } | Tone::Source | Tone::Marked => theme.text.fg,
            Tone::Struck | Tone::Raised | Tone::Lowered => theme.text.fg,
            Tone::Footnote => theme.accent,
            Tone::Callout(kind) => callout_colour(kind, theme),
            Tone::Inserted => theme.inserted(),
            Tone::Deleted | Tone::Failed => theme.deleted(),
            Tone::Margin(_) => theme.gutter.fg,
        };
        let selected = |at: Option<usize>| {
            at.is_some_and(|at| {
                self.selections
                    .iter()
                    .any(|s| at >= s.start() && at < s.end())
            })
        };
        let found = |at: Option<usize>| {
            at.is_some_and(|at| self.found.iter().any(|f| at >= f.0 && at < f.1))
        };
        let quoted = |at: Option<usize>| {
            at.is_some_and(|at| self.quoted.borrow().iter().any(|q| at >= q.0 && at < q.1))
        };
        let mark = |at: Option<usize>| {
            if selected(at) {
                Some(theme.selection)
            } else if found(at) {
                Some(theme.found)
            } else {
                quoted(at).then_some(theme.pick)
            }
        };
        let issues = self.current_issues();
        let flagged = |at: Option<usize>| {
            at.is_some_and(|at| issues.iter().any(|i| at >= i.start && at < i.end))
        };
        // Cells carry the text for the cell tier, and for tests. Pixels come from paint.
        for g in &row.glyphs {
            let style = cellix_render::Style {
                fg: fg_of(g.tone),
                bg: mark(g.at).filter(|_| !proportional).unwrap_or(bg),
                bold: g.font.bold,
                italic: g.font.italic,
                underline: g.underline || flagged(g.at),
                painted: proportional,
            };
            let x = left + g.x as u16;
            grid.put_str(x, y, g.ch.encode_utf8(&mut [0; 4]), style, area.right());
        }
        if !proportional {
            return;
        }
        let text_y = y as f32 + if row.tall { TALL_DROP } else { 0.0 };
        let right = area.right() as f32;
        for issue in issues {
            let inside = |g: &&Glyph| g.at.is_some_and(|at| at >= issue.start && at < issue.end);
            let mut word = row.glyphs.iter().filter(inside);
            let Some(first) = word.next() else {
                continue;
            };
            let last = word.next_back().unwrap_or(first);
            let x = origin + first.x;
            let width = last.x + last.advance - first.x;
            let colour = if issue.spelling {
                theme.deleted()
            } else {
                theme.accent
            };
            // Dots, since a solid line means a link, or text a change added.
            const DOT: f32 = 0.18;
            const STEP: f32 = 0.4;
            for dot in 0..(width / STEP).ceil() as usize {
                let spot = Bounds::cells(x + dot as f32 * STEP, text_y + 0.9, DOT, 0.07);
                grid.panel(spot, 0, Some(colour), None);
            }
        }
        let mut i = 0;
        while i < row.glyphs.len() {
            let g = row.glyphs[i];
            let same = |o: &Glyph| {
                o.font == g.font
                    && o.tone == g.tone
                    && o.underline == g.underline
                    && mark(o.at) == mark(g.at)
                    && (g.tone != Tone::TableLine || o.x == g.x)
            };
            // A table's cells are apart, so a run stops at a jump in position.
            let mut end = i + 1;
            while end < row.glyphs.len() && same(&row.glyphs[end]) {
                let prior = row.glyphs[end - 1];
                if (prior.x + prior.advance - row.glyphs[end].x).abs() > 0.01 {
                    break;
                }
                end += 1;
            }
            let run = &row.glyphs[i..end];
            let text: String = run.iter().map(|g| g.ch).collect();
            let x = origin + g.x;
            let width: f32 = run.iter().map(|g| g.advance).sum();
            if let Some(color) = mark(g.at) {
                let bounds = Bounds::cells(x, text_y + 0.06, width, 0.88);
                grid.panel(bounds, 6, Some(color), None);
            }
            match g.tone {
                Tone::TableLine => {}
                Tone::Formula { index, .. } => {
                    if let Some(figure) = self.figures().get(index as usize) {
                        // Text sits on a baseline below the row's middle: stand on it.
                        let line = text_y + 0.5 + 0.35 * cellix_render::INTERFACE_SIZE;
                        let top = line - figure.baseline;
                        grid.paint.push(Paint::Image {
                            bounds: Bounds::cells(x, top, figure.width, figure.height),
                            clip: Bounds::cells(
                                area.x as f32,
                                area.y as f32,
                                area.w as f32,
                                area.h as f32,
                            ),
                            bitmap: figure.bitmap.clone(),
                            color: theme.text.fg,
                        });
                    }
                }
                Tone::Check(checked) => {
                    // Square on screen: a row is about twice as tall as a column is wide.
                    const SIDE: f32 = 0.56;
                    let cell = self.measure.cell_pixels().unwrap_or((1, 2));
                    let across = SIDE * cell.1 as f32 / cell.0.max(1) as f32;
                    let left = x + ((width - across) / 2.0).max(0.0);
                    let top = y as f32 + (1.0 - SIDE) / 2.0;
                    let bounds = Bounds::cells(left, top, across, SIDE);
                    let (fill, edge) = if checked {
                        (Some(theme.accent), Some(theme.accent))
                    } else {
                        (None, Some(theme.gutter.fg))
                    };
                    grid.panel(bounds, 22, fill, edge);
                    if checked {
                        grid.paint.push(cellix_render::Paint::Icon {
                            bounds,
                            icon: Icon::Check,
                            color: theme.button_text,
                        });
                    }
                }
                _ => {
                    if matches!(g.tone, Tone::Code | Tone::Source) {
                        let pill = Bounds::cells(x - 0.2, text_y + 0.14, width + 0.4, 0.72);
                        grid.panel(pill, 14, Some(theme.code_bg), None);
                    }
                    if g.tone == Tone::Marked {
                        let pill = Bounds::cells(x - 0.1, text_y + 0.1, width + 0.2, 0.8);
                        grid.panel(pill, 8, Some(theme.highlighter()), None);
                    }
                    let shift = match g.tone {
                        Tone::Raised | Tone::Footnote => -0.2,
                        Tone::Lowered => 0.2,
                        Tone::Margin(moved) => moved as f32 / 64.0,
                        _ => 0.0,
                    };
                    let inked = g.underline || g.tone == Tone::Inserted;
                    let color = (fg_of(g.tone), inked);
                    grid.text_run((x, text_y + shift), right, &text, color, g.font);
                    if matches!(g.tone, Tone::Struck | Tone::Deleted) {
                        let line = Bounds::cells(x, text_y + 0.5, width, 0.0);
                        grid.hairline(line, fg_of(g.tone));
                    }
                }
            }
            i = end;
        }
    }

    fn draw_highlights(
        &self,
        grid: &mut Grid,
        area: Rect,
        gutter: u16,
        theme: &Theme,
        registry: &Registry,
    ) {
        let Some(path) = self.buffer.path() else {
            return;
        };
        let revision = self.buffer.revision();
        let mut cache = self.spans.borrow_mut();
        if !cache.as_ref().is_some_and(|(r, _)| *r == revision) {
            let bytes = self.buffer.rope().len_bytes();
            if bytes <= MAX_INLINE_HIGHLIGHT {
                *cache = Some((revision, registry.highlight(path, &self.buffer.text())));
                self.spans_bytes.set(bytes);
            } else if self.asked.get() != revision {
                self.asked.set(revision);
                let worker = self.worker.get_or_init(Worker::default);
                worker.request(path, self.buffer.text(), revision);
            }
        }
        let Some((cached, spans)) = cache.as_ref() else {
            return;
        };
        let rope = self.buffer.rope();
        // Until the worker answers, text after the edit keeps its colours by moving with it.
        let moved = rope.len_bytes() as isize - self.spans_bytes.get() as isize;
        let edit = rope.char_to_byte(self.selections.primary().head.min(rope.len_chars()));
        let shifted: Vec<Span>;
        let spans = if *cached == revision || moved == 0 {
            spans
        } else {
            let shift = |at: usize| {
                if at >= edit.saturating_sub(moved.max(0) as usize) {
                    at.saturating_add_signed(moved).min(rope.len_bytes())
                } else {
                    at
                }
            };
            shifted = spans
                .iter()
                .map(|s| Span {
                    start: shift(s.start),
                    end: shift(s.end),
                    token: s.token,
                })
                .collect();
            &shifted
        };
        let rows = self.code_rows(area.w);
        let Some((first_line, last_line)) = self.lines_in_view(&rows, area) else {
            return;
        };
        let from = rope.line_to_byte(first_line);
        let to = rope.line_to_byte(last_line);
        let first = spans.partition_point(|s| s.end <= from);
        for span in spans[first..].iter().take_while(|s| s.start < to) {
            let fg = theme.token(span.token);
            let (mut at, end) = (rope.byte_to_char(span.start), rope.byte_to_char(span.end));
            while at < end {
                let line = rope.char_to_line(at);
                let line_end =
                    end.min(rope.line_to_char(line) + self.line_text(line).chars().count());
                if line >= first_line && line < last_line && at < line_end {
                    for (y, x_from, x_to) in self.pieces(area, &rows, at, line_end) {
                        let x = (area.x + gutter).saturating_add(x_from as u16);
                        let w = (x_to - x_from.min(x_to)) as u16;
                        let clipped = Rect::new(x, y, w.min(area.right().saturating_sub(x)), 1);
                        grid.set_style(clipped, |style| cellix_render::Style { fg, ..style });
                    }
                }
                let next = rope.len_lines().min(line + 1);
                if next == line + 1 && next < rope.len_lines() {
                    at = rope.line_to_char(next);
                } else {
                    break;
                }
            }
        }
    }

    /// The bracket pairs in the file, found again only after an edit or once its strings
    /// and comments are known.
    fn pairs_found(&self) -> Rc<Vec<Bracket>> {
        let revision = self.buffer.revision();
        let spans = self.spans.borrow();
        let known = spans.as_ref().is_some_and(|(r, _)| *r == revision);
        let key = (revision, known);
        if let Some((cached, found)) = self.brackets.borrow().as_ref()
            && *cached == key
        {
            return found.clone();
        }
        let rope = self.buffer.rope();
        let found = if rope.len_bytes() > brackets::MAX_BYTES {
            Vec::new()
        } else {
            let skip: Vec<(usize, usize)> = spans
                .as_ref()
                .filter(|_| known)
                .map(|(_, spans)| {
                    spans
                        .iter()
                        .filter(|s| matches!(s.token, Token::String | Token::Comment))
                        .map(|s| (rope.byte_to_char(s.start), rope.byte_to_char(s.end)))
                        .collect()
                })
                .unwrap_or_default();
            brackets::find(rope, &skip)
        };
        let found = Rc::new(found);
        *self.brackets.borrow_mut() = Some((key, found.clone()));
        found
    }

    /// Where a char is drawn in the text, as a cell, if it's in view.
    fn screen_of(&self, area: Rect, (gutter, rows): (u16, &Rows), at: usize) -> Option<(u16, u16)> {
        let line = self.buffer.rope().char_to_line(at);
        if !rows.shows(line) {
            return None;
        }
        let (row, column) = self.place_of(rows, at);
        let row = row.checked_sub(self.top)?;
        let column = column.checked_sub(self.left())?;
        let x = (area.x + gutter) as usize + column;
        let y = area.y as usize + row;
        (x < area.right() as usize && y < area.bottom() as usize).then_some((x as u16, y as u16))
    }

    /// Brackets coloured by how deep they are, and the pair the caret touches marked: a tint
    /// on every tier and a ring where there are pixels.
    fn draw_brackets(
        &self,
        grid: &mut Grid,
        area: Rect,
        gutter: u16,
        theme: &Theme,
        focused: bool,
    ) {
        let found = self.pairs_found();
        let rope = self.buffer.rope();
        let rows = self.code_rows(area.w);
        let Some((first, last)) = self.lines_in_view(&rows, area) else {
            return;
        };
        let from = rope.line_to_char(first);
        let to = rope.line_to_char(last.min(rope.len_lines()));
        let colours = theme.brackets();
        for bracket in brackets::between(&found, from, to) {
            let Some((x, y)) = self.screen_of(area, (gutter, &rows), bracket.at) else {
                continue;
            };
            let fg = if bracket.partner.is_some() {
                colours[bracket.depth % colours.len()]
            } else {
                theme.error
            };
            grid.set_style(Rect::new(x, y, 1, 1), |style| cellix_render::Style {
                fg,
                ..style
            });
        }
        if !focused {
            return;
        }
        let Some((open, close)) = brackets::pair_at(&found, self.selections.primary().head) else {
            return;
        };
        for at in [open, close] {
            let Some((x, y)) = self.screen_of(area, (gutter, &rows), at) else {
                continue;
            };
            if self.measure.in_cells() {
                let bg = theme.bracket_match();
                grid.set_style(Rect::new(x, y, 1, 1), |style| cellix_render::Style {
                    bg,
                    ..style
                });
            } else {
                // One rounded shape, clear of the row's edges: a see-through tint, so the
                // bracket shows through it, and a ring.
                let shape = cellix_render::Bounds::cells(x as f32, y as f32 + 0.11, 1.0, 0.78);
                let tint = (theme.selection, 110);
                grid.wash(shape, 10, tint, Some(theme.bracket_ring()));
            }
        }
    }

    fn draw_range(
        &self,
        grid: &mut Grid,
        area: Rect,
        (gutter, rows): (u16, &Rows),
        s: &Selection,
        bg: Rgb,
    ) {
        let rope = self.buffer.rope();
        let (first, last) = (rope.char_to_line(s.start()), rope.char_to_line(s.end()));
        let Some((top_line, end_line)) = self.lines_in_view(rows, area) else {
            return;
        };
        for line in first.max(top_line)..=last.min(end_line.saturating_sub(1)) {
            let start = rope.line_to_char(line);
            let line_end = start + self.line_text(line).chars().count();
            let pieces = self.pieces(area, rows, s.start().max(start), s.end().min(line_end));
            let count = pieces.len();
            for (n, (y, from, to)) in pieces.into_iter().enumerate() {
                // The line's end is selected too, shown as one more cell after its last row.
                let newline = line < last && n + 1 == count;
                let width = (to - from.min(to)).max(usize::from(newline));
                let x = (area.x + gutter) as usize + from;
                let w = width.min((area.right() as usize).saturating_sub(x));
                grid.set_style(Rect::new(x as u16, y, w as u16, 1), |style| {
                    cellix_render::Style { bg, ..style }
                });
            }
        }
    }

    /// The first line in view, and the line after the last.
    fn lines_in_view(&self, rows: &Rows, area: Rect) -> Option<(usize, usize)> {
        let first = rows.get(self.top)?.line;
        let last = (self.top + area.h as usize)
            .min(rows.len())
            .checked_sub(1)?;
        Some((first, rows.get(last)?.line + 1))
    }
}
