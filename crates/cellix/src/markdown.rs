use cellix_render::{Grid, Rect, Style};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::theme::Theme;

/// A line of styled runs.
type Line = Vec<(String, Style)>;

/// Draws a live preview, wrapped to `area`. `scroll` is how far through the source the
/// editor beside it is, from 0.0 to 1.0, so the two stay roughly aligned.
pub fn draw(grid: &mut Grid, area: Rect, source: &str, scroll: f32, theme: &Theme) {
    grid.fill(area, theme.text);
    let width = area.w.saturating_sub(4).max(1) as usize;
    let lines: Vec<Line> = render(source, theme)
        .into_iter()
        .flat_map(|line| wrap(line, width))
        .collect();
    let max_top = lines.len().saturating_sub(area.h as usize);
    let top = (max_top as f32 * scroll).round() as usize;
    for (row, line) in lines.iter().skip(top).enumerate() {
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }
        let mut x = area.x + 2;
        for (text, style) in line {
            x = grid.put_str(x, y, text, *style, area.right());
        }
    }
}

/// Splits a line on spaces to fit `width`. Words longer than the width are left to clip.
fn wrap(line: Line, width: usize) -> Vec<Line> {
    let mut out = Vec::new();
    let mut current: Line = Vec::new();
    let mut used = 0;
    for (text, style) in line {
        for word in text.split_inclusive(' ') {
            let length = word.chars().count();
            if used + length > width && used > 0 {
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push((word.to_string(), style));
            used += length;
        }
    }
    out.push(current);
    out
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    header_rows: usize,
}

struct Builder<'a> {
    theme: &'a Theme,
    lines: Vec<Line>,
    line: Line,
    styles: Vec<Style>,
    /// The next number for each open list, or None for a bulleted one.
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    in_code: bool,
    table: Option<Table>,
}

impl Builder<'_> {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or(self.theme.text)
    }

    fn push(&mut self, text: &str) {
        let style = self.style();
        if let Some(table) = &mut self.table {
            if let Some(cell) = table.rows.last_mut().and_then(|r| r.last_mut()) {
                cell.push_str(text);
            }
            return;
        }
        if self.line.is_empty() && self.quote_depth > 0 {
            let bar = Style {
                fg: self.theme.comment,
                ..self.theme.text
            };
            self.line.push(("▎ ".repeat(self.quote_depth), bar));
        }
        self.line.push((text.to_string(), style));
    }

    fn flush(&mut self) {
        if !self.line.is_empty() {
            self.lines.push(std::mem::take(&mut self.line));
        }
    }

    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|l| !l.is_empty()) {
            self.lines.push(Vec::new());
        }
    }

    fn inline(&mut self, f: impl Fn(Style) -> Style) {
        self.styles.push(f(self.style()));
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|c| {
                let cell = |r: &Vec<String>| r.get(c).map_or(0, |s| s.chars().count());
                table.rows.iter().map(cell).max().unwrap_or(0)
            })
            .collect();
        let rule = Style {
            fg: self.theme.comment,
            ..self.theme.text
        };
        for (i, row) in table.rows.iter().enumerate() {
            let header = i < table.header_rows;
            let style = Style {
                bold: header,
                ..self.theme.text
            };
            let mut line: Line = Vec::new();
            for (c, width) in widths.iter().enumerate() {
                if c > 0 {
                    line.push((" │ ".to_string(), rule));
                }
                let cell = row.get(c).map_or("", String::as_str);
                line.push((format!("{cell:<width$}"), style));
            }
            self.lines.push(line);
            if i + 1 == table.header_rows {
                let parts: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                self.lines.push(vec![(parts.join("─┼─"), rule)]);
            }
        }
        self.lines.push(Vec::new());
    }
}

fn render(source: &str, theme: &Theme) -> Vec<Line> {
    let mut b = Builder {
        theme,
        lines: Vec::new(),
        line: Vec::new(),
        styles: Vec::new(),
        lists: Vec::new(),
        quote_depth: 0,
        in_code: false,
        table: None,
    };
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { .. }) => b.inline(|s| Style {
                bold: true,
                fg: theme.heading,
                ..s
            }),
            Event::End(TagEnd::Heading(_)) => {
                b.styles.pop();
                b.blank();
            }
            Event::Start(Tag::Strong) => b.inline(|s| Style { bold: true, ..s }),
            Event::Start(Tag::Emphasis) => b.inline(|s| Style { italic: true, ..s }),
            Event::Start(Tag::Link { .. }) => b.inline(|s| Style {
                fg: theme.accent,
                underline: true,
                ..s
            }),
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {
                b.styles.pop();
            }
            Event::Start(Tag::CodeBlock(_)) => {
                b.in_code = true;
                b.inline(|s| Style {
                    fg: theme.string,
                    ..s
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                b.in_code = false;
                b.styles.pop();
                b.blank();
            }
            Event::Start(Tag::BlockQuote(_)) => b.quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                b.quote_depth -= 1;
                b.blank();
            }
            Event::Start(Tag::List(start)) => b.lists.push(start),
            Event::End(TagEnd::List(_)) => {
                b.lists.pop();
                if b.lists.is_empty() {
                    b.blank();
                }
            }
            Event::Start(Tag::Item) => {
                let indent = "  ".repeat(b.lists.len().saturating_sub(1));
                let marker = match b.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}. ", *number - 1)
                    }
                    _ => "• ".to_string(),
                };
                b.push(&format!("{indent}{marker}"));
            }
            Event::End(TagEnd::Item) => b.flush(),
            Event::End(TagEnd::Paragraph) => {
                if b.lists.is_empty() {
                    b.blank();
                } else {
                    b.flush();
                }
            }
            Event::Start(Tag::Table(_)) => b.table = Some(Table::default()),
            Event::End(TagEnd::Table) => b.finish_table(),
            Event::Start(Tag::TableHead) => {
                if let Some(table) = &mut b.table {
                    table.rows.push(Vec::new());
                    table.header_rows = 1;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(table) = &mut b.table {
                    table.rows.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(row) = b.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(String::new());
                }
            }
            Event::Text(text) if b.in_code => {
                for code in text.lines() {
                    b.push(&format!("  {code}"));
                    b.flush();
                }
            }
            Event::Text(text) => b.push(&text),
            Event::Code(text) => {
                b.inline(|s| Style {
                    fg: theme.string,
                    ..s
                });
                b.push(&text);
                b.styles.pop();
            }
            Event::SoftBreak | Event::HardBreak => b.push(" "),
            Event::Rule => {
                b.line.push((
                    "─".repeat(40),
                    Style {
                        fg: theme.comment,
                        ..theme.text
                    },
                ));
                b.blank();
            }
            _ => {}
        }
    }
    b.flush();
    b.lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn structure_survives() {
        let theme = Theme::default();
        let lines = render("# T\n\n- a\n- b\n\n```\ncode\n```\n", &theme);
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all, vec!["T", "", "• a", "• b", "", "  code", ""]);
        assert!(lines[0][0].1.bold);
    }

    #[test]
    fn numbered_lists_keep_their_numbers() {
        let lines = render("3. a\n4. b\n", &Theme::default());
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all[..2], ["3. a", "4. b"]);
    }

    #[test]
    fn inline_styles_apply_to_their_words() {
        let theme = Theme::default();
        let lines = render("plain **bold** `code`\n", &theme);
        let runs = &lines[0];
        assert!(!runs[0].1.bold);
        assert!(runs[1].1.bold);
        assert_eq!(runs[3].1.fg, theme.string);
    }

    #[test]
    fn tables_line_up() {
        let theme = Theme::default();
        let lines = render("| Path | Holds |\n|---|---|\n| a | long text |\n", &theme);
        let all: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(all[0], "Path │ Holds    ");
        assert_eq!(all[1], "─────┼──────────");
        assert_eq!(all[2], "a    │ long text");
    }

    #[test]
    fn long_lines_wrap_on_words() {
        let line = vec![("one two three".to_string(), Style::default())];
        let wrapped: Vec<String> = wrap(line, 8).iter().map(text).collect();
        assert_eq!(wrapped, vec!["one two ", "three"]);
    }
}
