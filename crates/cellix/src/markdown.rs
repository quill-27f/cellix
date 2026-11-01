use cellix_render::{Grid, Rect, Style};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::theme::Theme;

/// Draws a Markdown document as formatted text rather than source.
pub fn draw(grid: &mut Grid, area: Rect, source: &str, top: usize, theme: &Theme) {
    grid.fill(area, theme.text);
    let lines = render(source, theme);
    for (row, (text, style)) in lines.iter().skip(top).enumerate() {
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }
        grid.put_str(area.x + 2, y, text, *style, area.right());
    }
}

fn render(source: &str, theme: &Theme) -> Vec<(String, Style)> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut style = theme.text;
    let mut list_depth = 0usize;
    let mut in_code = false;
    let flush = |line: &mut String, style: Style, lines: &mut Vec<(String, Style)>| {
        if !line.is_empty() {
            lines.push((std::mem::take(line), style));
        }
    };
    for event in Parser::new(source) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                style = Style {
                    bold: true,
                    fg: theme.accent,
                    underline: level == HeadingLevel::H1,
                    ..theme.text
                };
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut line, style, &mut lines);
                lines.push((String::new(), theme.text));
                style = theme.text;
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                style = Style {
                    fg: theme.string,
                    ..theme.text
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                lines.push((String::new(), theme.text));
                style = theme.text;
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth -= 1;
                if list_depth == 0 {
                    lines.push((String::new(), theme.text));
                }
            }
            Event::Start(Tag::Item) => {
                line = format!("{}• ", "  ".repeat(list_depth.saturating_sub(1)));
            }
            Event::End(TagEnd::Item) => flush(&mut line, style, &mut lines),
            Event::End(TagEnd::Paragraph) => {
                flush(&mut line, style, &mut lines);
                if list_depth == 0 {
                    lines.push((String::new(), theme.text));
                }
            }
            Event::Text(text) if in_code => {
                for code in text.lines() {
                    lines.push((format!("  {code}"), style));
                }
            }
            Event::Text(text) | Event::Code(text) => line.push_str(&text),
            Event::SoftBreak | Event::HardBreak => line.push(' '),
            _ => {}
        }
    }
    flush(&mut line, style, &mut lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_survives() {
        let theme = Theme::default();
        let lines = render("# T\n\n- a\n- b\n\n```\ncode\n```\n", &theme);
        let text: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(text, vec!["T", "", "• a", "• b", "", "  code", ""]);
        assert!(lines[0].1.bold);
    }
}
