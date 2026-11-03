//! A document's headings, to see its shape and move around it.

use cellix_render::{Bounds, Grid, Rect, Style};

use crate::rendered::Dialect;
use crate::theme::Theme;

const HEADER_ROWS: u16 = 2;
const INDENT: u16 = 2;
const LEFT: u16 = 2;

/// One heading, at a line counting from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub line: usize,
    /// The file it's in, when that's an `\\input` file and not the one in view.
    pub file: Option<std::path::PathBuf>,
}

/// A heading's words, without what only the source needs.
fn plain(text: &str) -> String {
    let words: String = text.chars().filter(|c| !"*_`$".contains(*c)).collect();
    words.trim().to_string()
}

fn markdown(text: &str) -> Vec<Heading> {
    let mut fenced = false;
    let mut found = Vec::new();
    for (line, source) in text.lines().enumerate() {
        let trimmed = source.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if !fenced && (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            found.push(Heading {
                level: hashes as u8,
                title: plain(&trimmed[hashes..]),
                line,
                file: None,
            });
        }
    }
    found
}

fn latex(
    text: &str,
    folder: Option<&std::path::Path>,
    file: Option<&std::path::Path>,
    depth: usize,
) -> Vec<Heading> {
    const LEVELS: [(&str, u8); 5] = [
        ("\\chapter", 1),
        ("\\section", 2),
        ("\\subsection", 3),
        ("\\subsubsection", 4),
        ("\\paragraph", 5),
    ];
    let mut found = Vec::new();
    for (line, source) in text.lines().enumerate() {
        let code = source.split('%').next().unwrap_or("").trim_start();
        // An input file's headings stand where it's read in.
        if let Some(folder) = folder.filter(|_| depth < 8) {
            for command in ["\\input{", "\\include{"] {
                let Some(name) = code.strip_prefix(command).and_then(|r| r.split('}').next())
                else {
                    continue;
                };
                let mut path = folder.join(name.trim());
                if path.extension().is_none() {
                    path.set_extension("tex");
                }
                if let Ok(inner) = std::fs::read_to_string(&path) {
                    found.extend(latex(&inner, Some(folder), Some(&path), depth + 1));
                }
            }
        }
        for (command, level) in LEVELS {
            let Some(rest) = code.strip_prefix(command) else {
                continue;
            };
            let rest = rest.strip_prefix('*').unwrap_or(rest);
            let Some(title) = rest.strip_prefix('{') else {
                continue;
            };
            // To the brace that closes it, so a `\\label` after it stays out.
            let mut depth = 0;
            let end = title.char_indices().find(|(_, c)| match c {
                '{' => {
                    depth += 1;
                    false
                }
                '}' if depth == 0 => true,
                '}' => {
                    depth -= 1;
                    false
                }
                _ => false,
            });
            let title = end.map_or(title, |(at, _)| &title[..at]);
            found.push(Heading {
                level,
                title: plain(title),
                line,
                file: file.map(std::path::Path::to_path_buf),
            });
        }
    }
    found
}

/// The headings, with a paper's `\\input` files read from `folder` and their headings in place.
pub fn of_in(text: &str, dialect: Dialect, folder: Option<&std::path::Path>) -> Vec<Heading> {
    match dialect {
        Dialect::Markdown => markdown(text),
        Dialect::Latex => latex(text, folder, None, 0),
        Dialect::Data(_) => Vec::new(),
    }
}

/// The heading a line comes under.
pub fn current(headings: &[Heading], line: usize) -> Option<usize> {
    headings.iter().rposition(|h| h.line <= line)
}

/// The panel in the sidebar.
#[derive(Default)]
pub struct Outline {
    pub headings: Vec<Heading>,
    top: usize,
}

impl Outline {
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.headings.len().saturating_sub(1);
        self.top = self.top.saturating_add_signed(delta).min(last);
    }

    /// The heading at a row of the panel.
    pub fn at(&self, area: Rect, y: u16) -> Option<&Heading> {
        let row = y.checked_sub(area.y + HEADER_ROWS)? as usize;
        self.headings.get(self.top + row)
    }

    pub fn draw(
        &self,
        grid: &mut Grid,
        area: Rect,
        (here, opened): (Option<usize>, bool),
        hover: Option<(u16, u16)>,
        theme: &Theme,
    ) {
        grid.fill(area, theme.sidebar);
        let bold = Style {
            bold: true,
            ..theme.sidebar
        };
        grid.label(area.x + LEFT, area.y, "Outline", bold, area.right());
        let quiet = Style {
            fg: theme.gutter.fg,
            ..theme.sidebar
        };
        if self.headings.is_empty() {
            let y = area.y + HEADER_ROWS;
            let note = if opened {
                "No headings here yet"
            } else {
                "Open a document to see its headings"
            };
            grid.label(area.x + LEFT, y, note, quiet, area.right());
            return;
        }
        let first = self.headings.iter().map(|h| h.level).min().unwrap_or(1);
        let rows = area.h.saturating_sub(HEADER_ROWS) as usize;
        for (row, (index, heading)) in self
            .headings
            .iter()
            .enumerate()
            .skip(self.top)
            .take(rows)
            .enumerate()
        {
            let y = area.y + HEADER_ROWS + row as u16;
            let line = Rect::new(area.x, y, area.w, 1);
            let hovered = hover.is_some_and(|h| line.contains(h.0, h.1));
            let chosen = here == Some(index);
            if chosen || hovered {
                let fill = if chosen {
                    theme.selection
                } else {
                    theme.row_hover
                };
                grid.panel(Bounds::inset(line, 0.5, 0.06), 12, Some(fill), None);
            }
            let x = area.x + LEFT + (heading.level - first) as u16 * INDENT;
            let style = Style {
                bold: heading.level == first,
                ..theme.sidebar
            };
            grid.label(x, y, &heading.title, style, area.right().saturating_sub(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_papers_outline_has_its_input_files_and_no_labels() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("method.tex"), "\\section{Method}\n").unwrap();
        let text = "\\section{Introduction}\\label{sec:intro}\n\\input{method}\n\\section{End}\n";
        let found = of_in(text, Dialect::Latex, Some(dir.path()));
        let titles: Vec<&str> = found.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, ["Introduction", "Method", "End"]);
        assert_eq!(
            found[1].file.as_deref(),
            Some(dir.path().join("method.tex").as_path())
        );
    }

    #[test]
    fn markdown_headings_skip_code() {
        let text = "# One\n\n```sh\n# not this\n```\n\n## Two **bold**\n";
        let found = of_in(text, Dialect::Markdown, None);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].title, "Two bold");
        assert_eq!(found[1].level, 2);
        assert_eq!(found[1].line, 6);
    }

    #[test]
    fn a_paper_has_its_sections() {
        let text = "\\section{Intro}\\label{s}\n% \\section{Old}\n\\subsection*{Detail}\n";
        let found = of_in(text, Dialect::Latex, None);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].level, 2);
        assert_eq!(found[1].title, "Detail");
    }

    #[test]
    fn the_caret_is_under_the_last_heading_above_it() {
        let found = of_in("# A\n\ntext\n\n# B\n", Dialect::Markdown, None);
        assert_eq!(current(&found, 2), Some(0));
        assert_eq!(current(&found, 4), Some(1));
    }
}
