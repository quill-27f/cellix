//! Pictures as a document's source writes them, so the page, the web page and the PDF all
//! read a picture's size the same way. Markdown has no way to say how wide an image is, so
//! this uses the form Pandoc and many others read: `![alt](path){width=50%}`.

/// An image standing alone on a Markdown line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageLine<'a> {
    pub indent: usize,
    pub alt: &'a str,
    pub path: &'a str,
    /// Share of the page, in percent.
    pub width: Option<u32>,
}

fn percent(attributes: &str) -> Option<u32> {
    let value = attributes.trim().strip_prefix("width=")?;
    value.strip_suffix('%')?.trim().parse().ok()
}

pub fn image_line(line: &str) -> Option<ImageLine<'_>> {
    let text = line.trim();
    let (text, width) = match text.rsplit_once("){") {
        Some((image, attributes)) if attributes.ends_with('}') => (
            &text[..=image.len()],
            percent(attributes.trim_end_matches('}')),
        ),
        _ => (text, None),
    };
    let rest = text.strip_prefix("![")?.strip_suffix(')')?;
    let (alt, path) = rest.split_once("](")?;
    (!path.contains(' ')).then_some(ImageLine {
        indent: line.len() - line.trim_start().len(),
        alt,
        path,
        width,
    })
}

/// A Markdown image line, set to a share of the page.
pub fn set_width(line: &str, percent: u32) -> Option<String> {
    let image = image_line(line)?;
    let pad = " ".repeat(image.indent);
    Some(format!(
        "{pad}![{}]({}){{width={percent}%}}",
        image.alt, image.path
    ))
}

/// A Markdown image line with another file in it, keeping its words and size.
pub fn set_path(line: &str, path: &str) -> Option<String> {
    let image = image_line(line)?;
    let pad = " ".repeat(image.indent);
    let size = image
        .width
        .map_or_else(String::new, |w| format!("{{width={w}%}}"));
    Some(format!("{pad}![{}]({path}){size}", image.alt))
}

/// The share of the page an `\includegraphics` line asks for.
pub fn graphics_width(line: &str) -> Option<u32> {
    let from = line.find("\\includegraphics[")? + "\\includegraphics[".len();
    let options = &line[from..from + line[from..].find(']')?];
    let value = options
        .split(',')
        .find_map(|o| o.trim().strip_prefix("width="))?;
    let number = value
        .split('\\')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("1");
    Some((number.parse::<f32>().ok()? * 100.0).round() as u32)
}

/// An `\includegraphics` line, set to a share of the page.
pub fn set_graphics_width(line: &str, percent: u32) -> Option<String> {
    let at = line.find("\\includegraphics")?;
    let after = at + "\\includegraphics".len();
    let brace = after + line[after..].find('{')?;
    let share = f64::from(percent) / 100.0;
    let width = if percent == 100 {
        "width=\\linewidth".to_string()
    } else {
        format!("width={share:.2}\\linewidth")
    };
    Some(format!(
        "{}\\includegraphics[{width}]{}",
        &line[..at],
        &line[brace..]
    ))
}

/// An `\includegraphics` line with another file in it.
pub fn set_graphics_path(line: &str, path: &str) -> Option<String> {
    let at = line.find("\\includegraphics")?;
    let open = at + line[at..].find('{')?;
    let close = open + line[open..].find('}')?;
    Some(format!("{}{path}{}", &line[..=open], &line[close..]))
}

/// The document with each sized image written the way `write` says.
pub fn rewrite_sized(markdown: &str, write: impl Fn(&ImageLine, u32) -> String) -> String {
    let lines = markdown.lines().map(|line| {
        image_line(line)
            .and_then(|image| image.width.map(|width| write(&image, width)))
            .unwrap_or_else(|| line.to_string())
    });
    lines.collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_can_carry_its_width() {
        let image = image_line("![Loss](plots/loss.png){width=50%}").unwrap();
        assert_eq!(image.path, "plots/loss.png");
        assert_eq!(image.alt, "Loss");
        assert_eq!(image.width, Some(50));
        assert_eq!(image_line("![a](b.png)").unwrap().width, None);
        assert!(image_line("text ![a](b.png)").is_none());
        assert!(image_line("![a](b c.png)").is_none());
    }

    #[test]
    fn sizing_an_image_rewrites_only_its_size() {
        assert_eq!(
            set_width("  ![a](b.png)", 25).unwrap(),
            "  ![a](b.png){width=25%}"
        );
        assert_eq!(
            set_width("![a](b.png){width=25%}", 75).unwrap(),
            "![a](b.png){width=75%}"
        );
        assert_eq!(
            set_path("![a](b.png){width=25%}", "c.jpg").unwrap(),
            "![a](c.jpg){width=25%}"
        );
    }

    #[test]
    fn a_paper_says_its_width_its_own_way() {
        let line = "  \\includegraphics[width=0.8\\linewidth]{a.png}";
        assert_eq!(graphics_width(line), Some(80));
        assert_eq!(
            graphics_width("\\includegraphics[width=\\linewidth]{a}"),
            Some(100)
        );
        assert_eq!(graphics_width("\\includegraphics{a}"), None);
        assert_eq!(
            set_graphics_width(line, 50).unwrap(),
            "  \\includegraphics[width=0.50\\linewidth]{a.png}"
        );
        assert_eq!(
            set_graphics_width("\\includegraphics{a}", 100).unwrap(),
            "\\includegraphics[width=\\linewidth]{a}"
        );
        assert_eq!(
            set_graphics_path(line, "b.pdf").unwrap(),
            "  \\includegraphics[width=0.8\\linewidth]{b.pdf}"
        );
    }
}
