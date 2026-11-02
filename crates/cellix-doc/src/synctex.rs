//! SyncTeX: how a PDF's pages map back to the source lines that made them, and forward again.
//! The engine writes the map when asked, and this reads it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

/// SyncTeX counts in scaled points. A PDF point is this many.
const SCALED_POINTS: f32 = 65781.76;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Mark {
    file: u32,
    line: usize,
    /// In PDF points, from the page's top left.
    x: f32,
    y: f32,
}

/// The marks of one PDF.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map {
    files: HashMap<u32, PathBuf>,
    pages: Vec<Vec<Mark>>,
}

/// Reads `tag,line:x,y` after a record's letter.
fn mark(record: &str) -> Option<Mark> {
    let (place, position) = record.split_once(':')?;
    let (file, line) = place.split_once(',')?;
    let position = position.split(':').next()?;
    let (x, y) = position.split_once(',')?;
    Some(Mark {
        file: file.parse().ok()?,
        line: line.parse().ok()?,
        x: x.parse::<f32>().ok()? / SCALED_POINTS,
        y: y.parse::<f32>().ok()? / SCALED_POINTS,
    })
}

impl Map {
    /// Reads the map beside a PDF, if the engine wrote one.
    pub fn beside(pdf: &Path) -> Option<Self> {
        let bytes = std::fs::read(pdf.with_extension("synctex.gz")).ok()?;
        let mut text = String::new();
        GzDecoder::new(&bytes[..]).read_to_string(&mut text).ok()?;
        Some(Self::parse(&text, pdf.parent().unwrap_or(Path::new(""))))
    }

    pub fn parse(text: &str, folder: &Path) -> Self {
        let mut map = Self::default();
        let mut page: Option<usize> = None;
        for line in text.lines() {
            if let Some(input) = line.strip_prefix("Input:") {
                if let Some((tag, path)) = input.split_once(':')
                    && let Ok(tag) = tag.parse()
                    && !path.is_empty()
                {
                    map.files.insert(tag, folder.join(path));
                }
                continue;
            }
            let mut chars = line.chars();
            match chars.next() {
                Some('{') => {
                    page = chars.as_str().parse::<usize>().ok().map(|p| p - 1);
                    if let Some(index) = page
                        && map.pages.len() <= index
                    {
                        map.pages.resize(index + 1, Vec::new());
                    }
                }
                Some('}') => page = None,
                // Text and its neighbours, not the boxes that hold a whole page.
                Some('h' | 'x' | 'k' | 'g' | '$') => {
                    if let (Some(index), Some(found)) = (page, mark(chars.as_str())) {
                        map.pages[index].push(found);
                    }
                }
                _ => {}
            }
        }
        map
    }

    /// The source line nearest a place on a page, with points from its top left.
    pub fn to_source(&self, page: usize, x: f32, y: f32) -> Option<(PathBuf, usize)> {
        let near = |m: &&Mark| (m.y - y).abs() * 4.0 + (m.x - x).abs();
        let found = self
            .pages
            .get(page)?
            .iter()
            .min_by(|a, b| near(a).total_cmp(&near(b)))?;
        Some((self.files.get(&found.file)?.clone(), found.line))
    }

    /// Where a source line landed, as a page and points from its top.
    pub fn to_pdf(&self, file: &Path, line: usize) -> Option<(usize, f32)> {
        let name = file.file_name()?;
        let tag = *self
            .files
            .iter()
            .find(|(_, path)| path.file_name() == Some(name))?
            .0;
        let mut best: Option<(usize, usize, f32)> = None;
        for (page, marks) in self.pages.iter().enumerate() {
            for found in marks.iter().filter(|m| m.file == tag) {
                let distance = found.line.abs_diff(line);
                if best.is_none_or(|b| distance < b.0) {
                    best = Some((distance, page, found.y));
                }
            }
        }
        best.map(|(_, page, y)| (page, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "SyncTeX Version:1
Input:1:/w/main.tex
Input:2:
Input:3:/w/two.tex
Content:
!372
{1
[1,38:4736287,46220575:26673152,41484288,0
h1,6:8799519,8865055:0,0,0
k1,10:12701574,11617567:3902055
x1,12:9000000,20000000
}1
!100
{2
h3,4:9000000,9000000:0,0,0
}2
";

    #[test]
    fn a_click_finds_its_line() {
        let map = Map::parse(SAMPLE, Path::new("/w"));
        let (x, y) = (9000000.0 / SCALED_POINTS, 20000000.0 / SCALED_POINTS);
        let (file, line) = map.to_source(0, x, y).unwrap();
        assert_eq!(file, Path::new("/w/main.tex"));
        assert_eq!(line, 12);
        assert_eq!(
            map.to_source(1, 0.0, 0.0).unwrap().0,
            Path::new("/w/two.tex")
        );
    }

    #[test]
    fn a_line_finds_its_page() {
        let map = Map::parse(SAMPLE, Path::new("/w"));
        let (page, y) = map.to_pdf(Path::new("/w/two.tex"), 5).unwrap();
        assert_eq!(page, 1);
        assert!((y - 9000000.0 / SCALED_POINTS).abs() < 0.01);
        assert_eq!(map.to_pdf(Path::new("/w/main.tex"), 11).unwrap().0, 0);
    }
}
