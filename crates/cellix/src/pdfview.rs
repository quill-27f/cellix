//! The Preview: a PDF beside its source, or an image. Pages render on a thread, one at a
//! time, so a long paper shows its first page at once and the interface never waits.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use cellix_doc::pdf::{Document, Page};
use cellix_doc::synctex::Map;
use cellix_render::{Bitmap, Bounds, Grid, Paint, Pixels, Rect, Style};

use crate::theme::Theme;

/// Space between pages, and around them, in rows and columns.
const GAP: f32 = 0.6;
const SIDE: f32 = 1.0;

/// How much one press of the zoom buttons changes the size.
const ZOOM_STEP: f32 = 0.25;
const ZOOM_RANGE: (f32, f32) = (0.5, 3.0);
/// Columns a notch of the wheel moves sideways. A column is about half as wide as a row is
/// tall, so this keeps pace with the three rows a notch moves down.
pub const SIDEWAYS: f32 = 6.0;
/// The scrollbars' thumbs: as thick as the editor's, and as far from the edge.
const THUMB: f32 = 0.5;
const THUMB_INSET: f32 = 0.2;
/// Rows between the bottom thumb and the edge under it.
const BOTTOM_GAP: f32 = 0.25;
/// Columns kept clear at either end of a scrollbar's track.
const END_GAP: f32 = 0.6;

/// What a click in the view lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    ZoomOut,
    ZoomIn,
    Fit,
    /// The thumb under the pages, which moves them sideways, and the one at the right, which
    /// moves them up and down.
    Sideways,
    Upright,
    /// A place on a page, in PDF points from its top left.
    Page(usize, f32, f32),
}

enum Update {
    /// The size of each page, in points.
    Sizes(Vec<(f32, f32)>),
    Done(usize, Page),
    Failed(String),
}

pub struct PdfView {
    pub path: PathBuf,
    pages: Vec<Option<(Bitmap, f32, f32)>>,
    sizes: Vec<(f32, f32)>,
    /// 1.0 fits the pane's width.
    zoom: f32,
    /// Maps pages to source lines, where the engine wrote one.
    sync: Option<Map>,
    updates: Option<Receiver<Update>>,
    /// The size pages were asked for: their width in cells, and a cell's pixels.
    fit: (u16, (u32, u32)),
    /// Rows the pages have under the bar, so they scroll no further than they need to.
    rows: u16,
    pub top: f32,
    /// How far the pages have moved sideways, in columns, when they're wider than the view.
    pub left: f32,
    pub error: Option<String>,
    generation: u64,
    /// When the file last changed, as of the last load.
    seen: Option<std::time::SystemTime>,
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// An image as a page, no wider than `width` pixels.
fn image_page(path: &Path, width: u32) -> Result<Page, String> {
    let image = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let image = if image.width() > width {
        let height = image.height() * width / image.width();
        image::imageops::thumbnail(&image, width, height.max(1))
    } else {
        image
    };
    let (width, height) = image.dimensions();
    Ok(Page {
        width,
        height,
        rgba: std::sync::Arc::new(image.into_raw()),
    })
}

/// Where the zoom controls sit in the bar, as (minus, plus, fit) columns.
fn buttons(area: Rect) -> (u16, u16, u16) {
    let fit = area.right().saturating_sub(5);
    (fit.saturating_sub(10), fit.saturating_sub(3), fit)
}

impl PdfView {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            pages: Vec::new(),
            sizes: Vec::new(),
            zoom: 1.0,
            sync: None,
            updates: None,
            fit: (0, (0, 0)),
            rows: 0,
            top: 0.0,
            left: 0.0,
            error: None,
            generation: 0,
            seen: None,
        }
    }

    /// Whether this shows an image rather than a PDF.
    pub fn is_image(&self) -> bool {
        !self.path.extension().is_some_and(|e| e == "pdf")
    }

    /// What to call what's shown, in what the view says.
    fn noun(&self) -> &'static str {
        if self.is_image() { "image" } else { "PDF" }
    }

    /// Reads the file again and draws its pages `width` cells wide. Old pages stay until
    /// new ones replace them, so a rebuild doesn't flash.
    pub fn load(&mut self, width: u16, cell: (u32, u32)) {
        let (sender, updates) = mpsc::channel();
        let path = self.path.clone();
        self.fit = (width, cell);
        self.generation += 1;
        self.updates = Some(updates);
        self.error = None;
        let pixels = (width as f32 - SIDE * 2.0).max(1.0) * self.zoom;
        let pixels = pixels as u32 * cell.0;
        self.seen = modified(&self.path);
        self.sync = Map::beside(&self.path);
        std::thread::spawn(move || {
            if !path.extension().is_some_and(|e| e == "pdf") {
                let _ = match image_page(&path, pixels) {
                    Ok(page) => sender
                        .send(Update::Sizes(vec![(page.width as f32, page.height as f32)]))
                        .and_then(|()| sender.send(Update::Done(0, page))),
                    Err(e) => sender.send(Update::Failed(e)),
                };
                return;
            }
            let opened = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(Document::open);
            let document = match opened {
                Ok(document) => document,
                Err(e) => {
                    let _ = sender.send(Update::Failed(e));
                    return;
                }
            };
            let sizes = (0..document.pages()).filter_map(|p| document.size(p));
            let _ = sender.send(Update::Sizes(sizes.collect()));
            for index in 0..document.pages() {
                let Some(page) = document.render(index, pixels) else {
                    continue;
                };
                if sender.send(Update::Done(index, page)).is_err() {
                    return;
                }
            }
        });
    }

    /// Reloads if the file has changed, as a plot does while a run writes it.
    pub fn refresh(&mut self) {
        let (width, cell) = self.fit;
        if width > 0 && modified(&self.path) != self.seen {
            self.load(width, cell);
        }
    }

    /// Reloads if the pane has changed size.
    pub fn fit(&mut self, (width, height): (u16, u16), cell: (u32, u32)) {
        self.rows = height.saturating_sub(1);
        if (width, cell) != self.fit {
            self.load(width, cell);
        }
    }

    /// Takes in finished pages. Returns true if there's something new to show.
    pub fn poll(&mut self) -> bool {
        let Some(updates) = &self.updates else {
            return false;
        };
        let cell = self.fit.1;
        let mut changed = false;
        while let Ok(update) = updates.try_recv() {
            changed = true;
            match update {
                Update::Sizes(sizes) => {
                    self.pages.resize(sizes.len(), None);
                    self.sizes = sizes;
                }
                Update::Failed(e) => self.error = Some(e),
                Update::Done(index, page) => {
                    let bitmap = Bitmap {
                        id: self.generation << 16 | index as u64,
                        width: page.width,
                        height: page.height,
                        kind: Pixels::Rgba,
                        pixels: page.rgba.clone(),
                    };
                    let width = page.width as f32 / cell.0 as f32;
                    let height = page.height as f32 / cell.1 as f32;
                    if let Some(slot) = self.pages.get_mut(index) {
                        *slot = Some((bitmap, width, height));
                    }
                }
            }
        }
        if changed {
            self.pan(0.0, 0.0);
        }
        changed
    }

    #[cfg(test)]
    pub fn pages_ready(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    /// Changes the size, and draws the pages again at it.
    pub fn zoom(&mut self, hit: Hit) {
        let before = self.zoom;
        self.zoom = match hit {
            Hit::ZoomIn => self.zoom + ZOOM_STEP,
            Hit::ZoomOut => self.zoom - ZOOM_STEP,
            _ => 1.0,
        }
        .clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        // The same part of the page stays in the middle of the view as it grows.
        let grown = self.zoom / before;
        let (width, content) = (f32::from(self.fit.0), self.content_width());
        let middle = if self.wide(self.fit.0) {
            (self.left + width / 2.0) / content
        } else {
            0.5
        };
        self.left = (middle * content * grown - width / 2.0).max(0.0);
        self.top *= grown;
        let (width, cell) = self.fit;
        self.load(width, cell);
    }

    /// The source line under a place on a page.
    pub fn source_at(&self, page: usize, x: f32, y: f32) -> Option<(PathBuf, usize)> {
        self.sync.as_ref()?.to_source(page, x, y)
    }

    /// Scrolls to where a source line landed. Returns false if it's not known.
    pub fn show_source(&mut self, file: &Path, line: usize) -> bool {
        let Some((page, y)) = self.sync.as_ref().and_then(|map| map.to_pdf(file, line)) else {
            return false;
        };
        let above: f32 = self.pages[..page.min(self.pages.len())]
            .iter()
            .flatten()
            .map(|(_, _, height)| height + GAP)
            .sum();
        let within = match (self.pages.get(page), self.sizes.get(page)) {
            (Some(Some((_, _, height))), Some((_, points))) => y / points * height,
            _ => 0.0,
        };
        self.top = (above + within - 6.0).max(0.0);
        true
    }

    /// The page at the top of the view, counting from one.
    fn page_in_view(&self) -> usize {
        let mut y = 0.0;
        for (index, page) in self.pages.iter().flatten().enumerate() {
            y += page.2 + GAP;
            if y > self.top + 1.0 {
                return index + 1;
            }
        }
        self.pages.len().max(1)
    }

    /// How wide the pages stand, with room either side, in columns.
    fn content_width(&self) -> f32 {
        let widest = self.pages.iter().flatten().map(|p| p.1).fold(0.0, f32::max);
        widest + SIDE * 2.0
    }

    /// How tall the pages stand, with the gaps between them, in rows.
    fn content_height(&self) -> f32 {
        let pages = self.pages.iter().flatten();
        pages.map(|(_, _, height)| height + GAP).sum::<f32>() + GAP
    }

    /// Whether the pages are wider than a view `width` columns wide, so they move sideways.
    pub fn wide(&self, width: u16) -> bool {
        self.content_width() > f32::from(width) + 0.01
    }

    /// Whether the pages are taller than a view `rows` high.
    fn tall(&self, rows: u16) -> bool {
        self.content_height() > f32::from(rows)
    }

    /// Where a page `width` columns wide starts: in the middle while the pages fit, and
    /// moved by `left` once they don't.
    fn page_x(&self, area: Rect, width: f32) -> f32 {
        let room = f32::from(area.w);
        if !self.wide(area.w) {
            return f32::from(area.x) + (room - width) / 2.0;
        }
        f32::from(area.x) + (self.content_width() - width) / 2.0 - self.left
    }

    /// Moves the pages by columns and rows, as far as they go.
    pub fn pan(&mut self, columns: f32, rows: f32) {
        let most = (self.content_width() - f32::from(self.fit.0)).max(0.0);
        self.left = (self.left + columns).clamp(0.0, most);
        self.scroll(rows);
    }

    /// Puts the pages where a drag that started at `left` and `top` has taken them.
    pub fn place(&mut self, left: f32, top: f32) {
        self.left = left;
        self.top = top;
        self.pan(0.0, 0.0);
    }

    /// Follows the pointer along a scrollbar's track, keeping the thumb's middle under it.
    pub fn follow_bar(&mut self, bar: Hit, area: Rect, x: f32, y: u16) {
        let view = Rect::new(area.x, area.y + 1, area.w, area.h.saturating_sub(1));
        if bar == Hit::Sideways {
            let along = (x - f32::from(view.x)) / f32::from(view.w.max(1));
            let content = self.content_width();
            self.left = along * content - f32::from(view.w) / 2.0;
        } else {
            let along = (f32::from(y) + 0.5 - f32::from(view.y)) / f32::from(view.h.max(1));
            self.top = along * self.content_height() - f32::from(view.h) / 2.0;
        }
        self.pan(0.0, 0.0);
    }

    /// What's at a place in the view.
    pub fn hit(&self, area: Rect, x: u16, y: u16) -> Option<Hit> {
        if y == area.y {
            let (minus, plus, fit) = buttons(area);
            return if x == minus {
                Some(Hit::ZoomOut)
            } else if x == plus {
                Some(Hit::ZoomIn)
            } else if x >= fit && x < fit + 3 {
                Some(Hit::Fit)
            } else {
                None
            };
        }
        // The scrollbars stand over the pages' edges, as the editor's do over its text.
        if self.wide(area.w) && y + 1 == area.bottom() {
            return Some(Hit::Sideways);
        }
        if self.tall(area.h.saturating_sub(1)) && x + 1 == area.right() && y > area.y {
            return Some(Hit::Upright);
        }
        let mut top = area.y as f32 + 1.0 + GAP - self.top;
        for (index, page) in self.pages.iter().enumerate() {
            let Some((_, width, height)) = page else {
                continue;
            };
            if (y as f32) >= top && (y as f32) < top + height {
                let left = self.page_x(area, *width);
                let (points_x, points_y) = self.sizes.get(index).copied()?;
                // From the middle of the cell clicked, not its top left: the top of a row is
                // already most of a line of text above where the pointer was.
                let across = (x as f32 + 0.5 - left).max(0.0) / width * points_x;
                let down = (y as f32 + 0.5 - top) / height * points_y;
                return Some(Hit::Page(index, across, down));
            }
            top += height + GAP;
        }
        None
    }

    /// Scrolls down the pages, until the last one's bottom is in view. Pages that fit don't
    /// move at all.
    pub fn scroll(&mut self, rows: f32) {
        let most = if self.rows > 0 {
            self.content_height() - f32::from(self.rows)
        } else {
            self.content_height() - 4.0
        };
        self.top = (self.top + rows).clamp(0.0, most.max(0.0));
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, cell_tier: bool, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let left = area.x + 2;
        if let Some(error) = &self.error {
            let said = format!("This {} can't be shown.", self.noun());
            grid.label(left, area.y + 1, &said, theme.sidebar, area.right());
            grid.label(left, area.y + 2, error, quiet, area.right());
            return;
        }
        if cell_tier {
            let name = self.path.display().to_string();
            let said = if self.is_image() {
                "Image ready:"
            } else {
                "PDF ready:"
            };
            grid.label(left, area.y + 1, said, theme.sidebar, area.right());
            grid.label(left, area.y + 2, &name, quiet, area.right());
            return;
        }
        if self.pages.is_empty() {
            let said = format!("Reading the {}…", self.noun());
            grid.label(left, area.y + 1, &said, quiet, area.right());
            return;
        }
        let bar = Rect::new(area.x, area.y, area.w, 1);
        grid.fill(bar, theme.tab);
        let (minus, plus, fit) = buttons(area);
        let quiet_bar = Style {
            fg: theme.status.fg,
            ..theme.tab
        };
        // An image has no pages to count, so the bar names it instead.
        let count = if self.is_image() {
            let name = self.path.file_name().unwrap_or_default();
            name.to_string_lossy().into_owned()
        } else {
            format!("Page {} of {}", self.page_in_view(), self.pages.len())
        };
        // Under the tab's own name, which starts half a cell in.
        let under_tab = area.x as f32 + 1.5;
        let end = minus.saturating_sub(1);
        grid.label_at(under_tab, left, area.y, &count, quiet_bar, end, 100);
        grid.label(minus, area.y, "−", quiet_bar, minus + 1);
        let size = format!("{:.0}%", self.zoom * 100.0);
        grid.label(minus + 2, area.y, &size, quiet_bar, plus);
        grid.label(plus, area.y, "+", quiet_bar, plus + 1);
        grid.label(fit, area.y, "Fit", quiet_bar, area.right());
        let area = Rect::new(area.x, area.y + 1, area.w, area.h.saturating_sub(1));
        // Pages scroll under the bar's own margin, not up against its text.
        let clip = Bounds::cells(
            area.x as f32,
            area.y as f32 + GAP,
            area.w as f32,
            area.h as f32 - GAP,
        );
        let mut y = area.y as f32 + GAP - self.top;
        for page in self.pages.iter().flatten() {
            let (bitmap, width, height) = page;
            if y + height > area.y as f32 && y < area.bottom() as f32 {
                let x = self.page_x(area, *width);
                let bounds = Bounds::cells(x, y, *width, *height);
                grid.paint.push(Paint::Image {
                    bounds,
                    clip,
                    bitmap: bitmap.clone(),
                    color: theme.text.fg,
                });
                draw_edge(grid, (x, y, *width, *height), area, theme);
            }
            y += height + GAP;
        }
        self.draw_thumbs(grid, area, theme);
    }

    /// Thin thumbs over the pages' bottom and right edges, while there's more to see.
    fn draw_thumbs(&self, grid: &mut Grid, area: Rect, theme: &Theme) {
        let (cw, ch) = self.fit.1;
        // A column is narrower than a row is tall, so the bottom thumb is thinner in rows.
        let aspect = if ch > 0 { cw as f32 / ch as f32 } else { 0.5 };
        let (x, y) = (f32::from(area.x), f32::from(area.y));
        let (w, h) = (f32::from(area.w), f32::from(area.h));
        if self.wide(area.w) {
            let content = self.content_width();
            let thick = THUMB * aspect;
            // Its track stops short of the view's corners, clear of the edges.
            let track = w - 2.0 * END_GAP;
            let long = (track * w / content).max(2.0);
            let start = x + END_GAP + self.left / content * track;
            // Clear of the status bar under it by a quarter of a row.
            let top = y + h - BOTTOM_GAP - thick;
            let bounds = Bounds::cells(start, top, long, thick);
            grid.panel(bounds, (thick * 50.0) as u8, Some(theme.scrollbar), None);
        }
        if self.tall(area.h) && area.h > 0 {
            let content = self.content_height();
            let track = h - 2.0 * END_GAP * aspect;
            let long = (track * h / content).max(1.0);
            let start = y + END_GAP * aspect + (self.top / content * track).min(track - long);
            let left = x + w - THUMB_INSET - THUMB;
            let bounds = Bounds::cells(left, start, THUMB, long);
            grid.panel(
                bounds,
                (THUMB * aspect * 50.0) as u8,
                Some(theme.scrollbar),
                None,
            );
        }
    }
}

/// A page's outline, as far as it shows: a page moved past the view's edge mustn't draw its
/// outline over what stands beside the view.
fn draw_edge(grid: &mut Grid, (x, y, w, h): (f32, f32, f32, f32), area: Rect, theme: &Theme) {
    let (left, top) = (f32::from(area.x), f32::from(area.y));
    let (right, bottom) = (f32::from(area.right()), f32::from(area.bottom()));
    let (x0, x1) = (x.max(left), (x + w).min(right));
    let (y0, y1) = (y.max(top), (y + h).min(bottom));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let colour = theme.border;
    if y >= top {
        grid.hairline(Bounds::cells(x0, y, x1 - x0, 0.0), colour);
    }
    if y + h <= bottom {
        grid.hairline(Bounds::cells(x0, y + h, x1 - x0, 0.0), colour);
    }
    if x >= left {
        grid.hairline(Bounds::cells(x, y0, 0.0, y1 - y0), colour);
    }
    if x + w <= right {
        grid.hairline(Bounds::cells(x + w, y0, 0.0, y1 - y0), colour);
    }
}
