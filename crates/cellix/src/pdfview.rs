//! A PDF beside its source. Pages render on a thread, one at a time, so a long paper shows
//! its first page at once and the interface never waits.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use cellix_doc::pdf::{Document, Page};
use cellix_render::{Bitmap, Bounds, Grid, Paint, Pixels, Rect, Style};

use crate::theme::Theme;

/// Space between pages, and around them, in rows and columns.
const GAP: f32 = 0.6;
const SIDE: f32 = 1.0;

enum Update {
    /// How many pages there are.
    Count(usize),
    Done(usize, Page),
    Failed(String),
}

pub struct PdfView {
    pub path: PathBuf,
    pages: Vec<Option<(Bitmap, f32, f32)>>,
    updates: Option<Receiver<Update>>,
    /// The size pages were asked for: their width in cells, and a cell's pixels.
    fit: (u16, (u32, u32)),
    pub top: f32,
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

impl PdfView {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            pages: Vec::new(),
            updates: None,
            fit: (0, (0, 0)),
            top: 0.0,
            error: None,
            generation: 0,
            seen: None,
        }
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
        let pixels = (width as f32 - SIDE * 2.0).max(1.0) as u32 * cell.0;
        self.seen = modified(&self.path);
        std::thread::spawn(move || {
            if !path.extension().is_some_and(|e| e == "pdf") {
                let _ = match image_page(&path, pixels) {
                    Ok(page) => sender
                        .send(Update::Count(1))
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
            let _ = sender.send(Update::Count(document.pages()));
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
    pub fn fit(&mut self, width: u16, cell: (u32, u32)) {
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
                Update::Count(count) => self.pages.resize(count, None),
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
        changed
    }

    #[cfg(test)]
    pub fn pages_ready(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    pub fn scroll(&mut self, rows: f32) {
        let total: f32 = self
            .pages
            .iter()
            .flatten()
            .map(|(_, _, height)| height + GAP)
            .sum();
        self.top = (self.top + rows).clamp(0.0, (total - 4.0).max(0.0));
    }

    pub fn draw(&self, grid: &mut Grid, area: Rect, cell_tier: bool, theme: &Theme) {
        grid.fill(area, theme.sidebar);
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        let left = area.x + 2;
        if let Some(error) = &self.error {
            grid.label(
                left,
                area.y + 1,
                "This PDF can't be shown.",
                theme.sidebar,
                area.right(),
            );
            grid.label(left, area.y + 2, error, quiet, area.right());
            return;
        }
        if cell_tier {
            let name = self.path.display().to_string();
            grid.label(left, area.y + 1, "PDF ready:", theme.sidebar, area.right());
            grid.label(left, area.y + 2, &name, quiet, area.right());
            return;
        }
        if self.pages.is_empty() {
            grid.label(left, area.y + 1, "Reading the PDF…", quiet, area.right());
            return;
        }
        let clip = Bounds::cells(area.x as f32, area.y as f32, area.w as f32, area.h as f32);
        let mut y = area.y as f32 + GAP - self.top;
        for page in self.pages.iter().flatten() {
            let (bitmap, width, height) = page;
            if y + height > area.y as f32 && y < area.bottom() as f32 {
                let x = area.x as f32 + (area.w as f32 - width) / 2.0;
                let bounds = Bounds::cells(x, y, *width, *height);
                grid.paint.push(Paint::Image {
                    bounds,
                    clip,
                    bitmap: bitmap.clone(),
                    color: theme.text.fg,
                });
                let edge = Bounds::cells(x, y, *width, *height);
                grid.panel(edge, 0, None, Some(theme.border));
            }
            y += height + GAP;
        }
    }
}
