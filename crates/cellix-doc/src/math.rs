//! Typeset math, from pure Rust and fonts in the binary, so it works over SSH on a server
//! with no TeX. A formula that won't parse is an error here and its source on the page.

use std::cell::OnceCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use latex_rust::{
    Dim, MathBox, MathFont, MathStyle, PngOptions, SvgOptions, latex_to_svg,
    layout_with_em_size_pt, parse, render_png,
};

/// A typeset formula as coverage, so it can take the colour of the text around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formula {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// Rows of pixels above the baseline.
    pub baseline: u32,
    pub alpha: Arc<Vec<u8>>,
}

thread_local! {
    static FONT: OnceCell<Option<MathFont>> = const { OnceCell::new() };
}

fn with_font<T>(run: impl FnOnce(&MathFont) -> Result<T, String>) -> Result<T, String> {
    FONT.with(|cell| {
        let font = cell.get_or_init(|| MathFont::stix_two_math().ok());
        run(font.as_ref().ok_or("the math font didn't load")?)
    })
}

/// Environments that only number or wrap math, which the page has no use for.
const WRAPPERS: [&str; 4] = ["equation", "equation*", "displaymath", "math"];

/// Commands the engine doesn't have, as what it does. Longest names first where one is the
/// start of another. The page is a preview, and a near match beats losing a whole display.
const SUBSTITUTES: [(&str, &str); 24] = [
    // In `\operatorname` a thin space prints as a comma, so these stay in math.
    ("\\operatorname*{", "\\mathrm{"),
    ("\\operatorname{arg\\,", "\\mathrm{arg\\,"),
    ("\\argmin", "\\mathrm{arg\\,min}"),
    ("\\argmax", "\\mathrm{arg\\,max}"),
    ("\\bm{", "\\mathbf{"),
    ("\\boldsymbol{", "\\mathbf{"),
    ("\\mathds{", "\\mathbb{"),
    ("\\coloneqq", ":="),
    ("\\eqqcolon", "=:"),
    ("\\varinjlim", "\\lim"),
    ("\\varprojlim", "\\lim"),
    ("\\xmapsto", "\\xrightarrow"),
    ("\\llbracket", "[\\!["),
    ("\\rrbracket", "]\\!]"),
    ("\\bmod", "\\operatorname{mod}"),
    ("\\qed", "\\square"),
    ("\\middle", ""),
    ("\\displaystyle", ""),
    ("\\textstyle", ""),
    ("\\nonumber", ""),
    ("\\notag", ""),
    ("{dcases}", "{cases}"),
    ("{rcases}", "{cases}"),
    ("{smallmatrix}", "{matrix}"),
];

/// Operators whose limits stand under them in a display. The engine only does this for `\lim`.
const UNDER: [&str; 8] = [
    "\\max",
    "\\min",
    "\\sup",
    "\\inf",
    "\\limsup",
    "\\liminf",
    "\\mathrm{arg\\,min}",
    "\\mathrm{arg\\,max}",
];

/// A script at the start of `text`: its text, and how far it runs.
fn script(text: &str) -> Option<(&str, usize)> {
    let mut chars = text.char_indices();
    match chars.next()?.1 {
        '{' => {
            let mut depth = 0;
            for (at, c) in text.char_indices() {
                depth += match c {
                    '{' => 1,
                    '}' => -1,
                    _ => 0,
                };
                if depth == 0 {
                    return Some((&text[1..at], at + 1));
                }
            }
            None
        }
        '\\' => {
            let end = chars
                .find(|(_, c)| !c.is_ascii_alphabetic())
                .map_or(text.len(), |(at, _)| at);
            Some((&text[..end], end))
        }
        c => Some((&text[..c.len_utf8()], c.len_utf8())),
    }
}

/// Sets `\max_{x}` as a stack, so its limit stands under it the way TeX puts it.
fn limits_under(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let found = UNDER.iter().find(|name| {
            rest.strip_prefix(**name)
                .is_some_and(|after| !after.starts_with(|c: char| c.is_ascii_alphabetic()))
        });
        let stacked = found.and_then(|name| {
            let after = rest[name.len()..].trim_start();
            let after = after.strip_prefix("\\limits").unwrap_or(after).trim_start();
            let (under, used) = script(after.strip_prefix('_')?.trim_start())?;
            let spaces = after.len() - 1 - after[1..].trim_start().len();
            let tail = &after[1 + spaces + used..];
            Some((format!("\\underset{{{under}}}{{{name}}}"), tail))
        });
        match stacked {
            Some((stack, tail)) => {
                out.push_str(&stack);
                rest = tail;
            }
            None => {
                let step = found.map_or(1, |name| name.len());
                out.push_str(&rest[..step]);
                rest = &rest[step..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Takes `\name{...}` out of `text`, with `keep` deciding what stands in its place.
fn strip_command(text: &mut String, name: &str, keep: bool) {
    let open = format!("{name}{{");
    while let Some(at) = text.find(&open) {
        let start = at + open.len();
        let mut depth = 1;
        let mut end = None;
        for (i, c) in text[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = Some(start + i);
                break;
            }
        }
        let Some(end) = end else {
            return;
        };
        let inner = text[start..end].to_string();
        text.replace_range(at..=end, if keep { &inner } else { "" });
    }
}

/// What the math engine reads: `\label` has no picture, and `equation` only numbers.
fn prepared(source: &str, display: bool) -> String {
    let mut text = source.trim().to_string();
    for name in WRAPPERS {
        let (open, close) = (format!("\\begin{{{name}}}"), format!("\\end{{{name}}}"));
        if let Some(inner) = text
            .strip_prefix(&open)
            .and_then(|t| t.strip_suffix(&close))
        {
            text = inner.trim().to_string();
        }
    }
    // Numbers belong in the margin, where the page puts them.
    strip_command(&mut text, "\\label", false);
    strip_command(&mut text, "\\tag", false);
    strip_command(&mut text, "\\tag*", false);
    strip_command(&mut text, "\\smash", true);
    strip_command(&mut text, "\\mathop", true);
    strip_command(&mut text, "\\mathrel", true);
    strip_command(&mut text, "\\mathbin", true);
    for (from, to) in SUBSTITUTES {
        text = text.replace(from, to);
    }
    // Numbered environments number themselves, and restart in every block, so they all go
    // through the one that doesn't.
    for name in [
        "align", "gather", "multline", "alignat", "flalign", "eqnarray",
    ] {
        text = text
            .replace(&format!("{{{name}*}}"), "{aligned}")
            .replace(&format!("{{{name}}}"), "{aligned}");
    }
    let text = text.replace("{gathered}", "{aligned}");
    if display { limits_under(&text) } else { text }
}

fn pixels(dim: &Dim, em: u32) -> f64 {
    let (top, bottom) = dim.as_ratio().unwrap_or((0, 1));
    top as f64 / bottom.max(1) as f64 * em as f64
}

fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    let alpha = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| p[channels - 1])
        .collect();
    Ok((info.width, info.height, alpha))
}

/// Typesets a formula with `em` pixels to the em.
pub fn render(source: &str, display: bool, em: u32) -> Result<Formula, String> {
    let text = prepared(source, display);
    if text.is_empty() {
        return Err("empty formula".to_string());
    }
    let ast = parse(&text).map_err(|e| e.to_string())?;
    with_font(|font| {
        let style = if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        };
        let size = Dim::from_i64(i64::from(em));
        let tree: MathBox =
            layout_with_em_size_pt(&ast, font, style, &size).map_err(|e| e.to_string())?;
        // At 72 dpi a point is a pixel, so the em size is exact.
        let mut options = PngOptions::new();
        options.font_size_pt = size;
        options.dpi = Dim::from_i64(72);
        let bytes = render_png(&tree, font, &options).map_err(|e| e.to_string())?;
        let (width, height, alpha) = decode(&bytes)?;
        let mut hasher = DefaultHasher::new();
        (&text, display, em).hash(&mut hasher);
        Ok(Formula {
            id: hasher.finish(),
            width,
            height,
            baseline: pixels(&tree.height, em).round() as u32,
            alpha: Arc::new(alpha),
        })
    })
}

/// A formula as self-contained SVG, for pages that leave Cellix.
pub fn svg(source: &str, display: bool) -> Result<String, String> {
    let text = prepared(source, display);
    with_font(|font| {
        let mut options = SvgOptions::new();
        options.display = display;
        let svg = latex_to_svg(&text, font, &options).map_err(|e| e.to_string())?;
        // The XML header would land in the middle of a page.
        Ok(svg
            .find("<svg")
            .map_or(svg.clone(), |at| svg[at..].to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_stand_under_operators_in_a_display() {
        let beside = render(r"\max_{x \in X} f", false, 30).unwrap();
        let under = render(r"\max_{x \in X} f", true, 30).unwrap();
        assert!(under.height > beside.height + 10);
        let wide = render(r"\argmin_{\theta} f + \min_x g + \max^2", true, 30).unwrap();
        assert!(wide.height > beside.height + 10);
        assert_eq!(
            prepared(r"\max_{x}^2 \maximum", true),
            r"\underset{x}{\max}^2 \maximum"
        );
        assert_eq!(prepared(r"\min\limits_a b", true), r"\underset{a}{\min} b");
    }

    #[test]
    fn a_thin_space_in_an_operators_name_is_not_a_comma() {
        let text = prepared(r"\operatorname{arg\,min} + \operatorname*{arg\,max}", false);
        assert!(!text.contains("operatorname"), "{text}");
    }

    #[test]
    fn asking_for_limits_never_breaks_a_formula() {
        assert!(render(r"\sum\limits_{i=1}^n x", false, 30).is_ok());
        assert!(render(r"\sum\nolimits_{i=1}^n x", true, 30).is_ok());
    }

    #[test]
    fn a_formula_comes_out_as_real_coverage() {
        let formula = render(r"\frac{a}{b} + \sum_{i=1}^{n} x_i^2", false, 22).unwrap();
        assert!(formula.width > 22 && formula.height > 10);
        assert_eq!(
            formula.alpha.len(),
            (formula.width * formula.height) as usize
        );
        assert!(formula.alpha.iter().any(|a| *a > 200), "ink");
        assert!(formula.alpha.contains(&0), "and paper");
        assert!(formula.baseline > 0 && formula.baseline < formula.height);
    }

    #[test]
    fn display_math_stands_taller_than_inline() {
        let text = r"\sum_{i=1}^{n} i";
        let inline = render(text, false, 22).unwrap();
        let display = render(text, true, 22).unwrap();
        assert!(display.height > inline.height);
    }

    #[test]
    fn scientific_environments_typeset() {
        for source in [
            r"\begin{align} a &= b \\ c &= d \end{align}",
            r"\begin{equation} E = mc^2 \label{eq:e} \end{equation}",
            r"\begin{pmatrix} 1 & 0 \\ 0 & 1 \end{pmatrix}",
            r"f(x) = \begin{cases} 0 & x < 0 \\ 1 & x \ge 0 \end{cases}",
            r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
            r"\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}",
        ] {
            render(source, true, 22).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }

    #[test]
    fn what_the_engine_lacks_has_a_stand_in() {
        for source in [
            r"\operatorname*{arg\,max}_x f(x)",
            r"\argmin_{\theta} \mathcal{L}(\theta)",
            r"\bm{x}^\top \mathds{1}",
            r"a \coloneqq b",
            r"\begin{dcases} a \\\\ b \end{dcases}",
            r"x \bmod y",
            r"\begin{align} a &= b \tag{2} \\\\ c &= d \notag \end{align}",
        ] {
            render(source, true, 22).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }

    #[test]
    fn numbers_are_left_to_the_margin() {
        assert_eq!(prepared(r"E = mc^2 \tag{2}", true).trim(), "E = mc^2");
        assert!(prepared(r"\begin{align} a \end{align}", true).contains("{aligned}"));
    }

    #[test]
    fn nonsense_is_an_error_not_a_picture() {
        assert!(render(r"\frac{", false, 22).is_err());
        assert!(render("  ", false, 22).is_err());
    }

    #[test]
    fn svg_stands_alone() {
        assert!(svg("x^2", false).unwrap().starts_with("<svg"));
    }
}
