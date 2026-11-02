//! A PDF's pages as pixels, from pure Rust, so a paper shows beside its source on any host.

use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

/// One page, as straight RGBA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// An open PDF. Its pages render on demand.
pub struct Document {
    pdf: Pdf,
}

impl Document {
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let pdf = Pdf::new(bytes).map_err(|e| format!("{e:?}"))?;
        Ok(Self { pdf })
    }

    pub fn pages(&self) -> usize {
        self.pdf.pages().len()
    }

    /// A page as (width, height) in points.
    pub fn size(&self, page: usize) -> Option<(f32, f32)> {
        let (width, height) = self.pdf.pages().get(page)?.render_dimensions();
        Some((width, height))
    }

    /// Draws a page at `scale` pixels to the point, or smaller if that would pass `max_width`.
    pub fn render_at(&self, page: usize, scale: f32, max_width: u32) -> Option<Page> {
        let (width, _) = self.size(page)?;
        self.render(page, ((width * scale) as u32).clamp(1, max_width.max(1)))
    }

    /// Draws a page `width` pixels wide.
    pub fn render(&self, page: usize, width: u32) -> Option<Page> {
        let pages = self.pdf.pages();
        let page = pages.get(page)?;
        let (natural, _) = page.render_dimensions();
        let scale = width as f32 / natural.max(1.0);
        let pixmap = render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let (width, height) = (pixmap.width() as u32, pixmap.height() as u32);
        // A white background leaves nothing see-through, so premultiplying changed nothing.
        let rgba = pixmap.data_as_u8_slice().to_vec();
        Some(Page {
            width,
            height,
            rgba: Arc::new(rgba),
        })
    }
}

/// The smallest PDF that shows something, for tests.
#[cfg(any(test, feature = "fixtures"))]
pub fn sample() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>",
    ];
    let stream = "0 0 1 rg 20 20 100 100 re f";
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    offsets.push(pdf.len());
    pdf.push_str(&format!(
        "4 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n",
        stream.len()
    ));
    let table = pdf.len();
    pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
    for offset in &offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n"
    ));
    pdf.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_comes_out_as_pixels() {
        let document = Document::open(sample()).unwrap();
        assert_eq!(document.pages(), 1);
        assert_eq!(document.size(0), Some((200.0, 300.0)));
        let page = document.render(0, 400).unwrap();
        assert_eq!((page.width, page.height), (400, 600));
        assert_eq!(page.rgba.len(), 400 * 600 * 4);
        let at = |x: usize, y: usize| {
            let i = (y * 400 + x) * 4;
            (page.rgba[i], page.rgba[i + 1], page.rgba[i + 2])
        };
        assert_eq!(at(10, 10), (255, 255, 255), "paper");
        assert_eq!(
            at(100, 500),
            (0, 0, 255),
            "and the square on it, at the bottom"
        );
    }

    #[test]
    fn nonsense_is_an_error() {
        assert!(Document::open(b"not a pdf".to_vec()).is_err());
    }
}
