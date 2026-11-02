//! A Markdown file as a web page that stands alone: math is inline SVG, so the page needs
//! no scripts and no network.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};

use crate::{diagram, figure, math};

const STYLE: &str = "
:root { color-scheme: light dark; --text: #1f2328; --dim: #59636e; --line: #d1d9e0;
  --code: #f6f8fa; --link: #0969da; }
@media (prefers-color-scheme: dark) { :root { --text: #e6edf3; --dim: #9198a1;
  --line: #3d444d; --code: #151b23; --link: #4493f8; }
  .math svg, .inline-math svg { filter: invert(1); }
  .diagram { background: #fff; border-radius: 8px; } }
body { max-width: 46rem; margin: 3rem auto; padding: 0 1.25rem; color: var(--text);
  font: 17px/1.65 -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
h1, h2, h3 { line-height: 1.25; margin: 2em 0 .6em; } h1 { margin-top: 0; }
a { color: var(--link); } img { max-width: 100%; }
code { font: .88em ui-monospace, Menlo, monospace; background: var(--code);
  padding: .15em .35em; border-radius: 5px; }
pre { background: var(--code); border: 1px solid var(--line); border-radius: 8px;
  padding: 1em; overflow: auto; } pre code { background: none; padding: 0; }
blockquote { margin: 1em 0; padding-left: 1em; border-left: 3px solid var(--line);
  color: var(--dim); }
table { border-collapse: collapse; margin: 1em 0; }
th, td { border: 1px solid var(--line); padding: .4em .8em; } th { background: var(--code); }
.math { text-align: center; margin: 1.2em 0; overflow-x: auto; }
.diagram { text-align: center; margin: 1.2em 0; padding: 1em; overflow-x: auto; }
.diagram svg { max-width: 100%; height: auto; }
mark { background: #ffec9e; color: #1f2328; padding: 0 .15em; border-radius: 3px; }
ins { color: #1a7f37; } del { color: #cf222e; } .note { color: var(--dim); font-size: .85em; }
blockquote[class^=markdown-alert] { color: inherit; border-left-width: 4px; }
blockquote[class^=markdown-alert]::before { display: block; font-weight: 600; margin-top: .6em; }
.markdown-alert-note { border-color: var(--link); } .markdown-alert-note::before { content: 'Note'; }
.markdown-alert-tip { border-color: #1a7f37; } .markdown-alert-tip::before { content: 'Tip'; }
.markdown-alert-important { border-color: #8857e0; }
.markdown-alert-important::before { content: 'Important'; }
.markdown-alert-warning { border-color: #c08610; }
.markdown-alert-warning::before { content: 'Warning'; }
.markdown-alert-caution { border-color: #cf222e; }
.markdown-alert-caution::before { content: 'Caution'; }
.footnote-definition { font-size: .88em; color: var(--dim); }
hr { border: 0; border-top: 1px solid var(--line); margin: 2em 0; }
";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn formula(source: &str, display: bool) -> String {
    let shown =
        math::svg(source, display).unwrap_or_else(|_| format!("<code>{}</code>", escape(source)));
    if display {
        format!("<div class=\"math\">{shown}</div>")
    } else {
        format!("<span class=\"inline-math\">{shown}</span>")
    }
}

/// A block only LaTeX can draw, as what `built` has for it, or as its source.
fn built_or_source(language: &str, source: &str, built: &Built) -> String {
    let body = diagram::fenced_tikz(language, source);
    match built(&body) {
        Some(picture) => format!("<div class=\"diagram\">{picture}</div>"),
        None => format!("<pre><code>{}</code></pre>", escape(source)),
    }
}

/// A diagram as a picture, or its source if it can't be drawn.
fn drawn(kind: diagram::Kind, source: &str) -> String {
    match diagram::svg(kind, source, false) {
        Ok(svg) => format!("<div class=\"diagram\">{svg}</div>"),
        Err(_) => format!("<pre><code>{}</code></pre>", escape(source)),
    }
}

/// `H~2~O` and `x^2^` as tags, leaving alone any tilde or caret that doesn't hug its text.
fn raised(line: &str, marker: char, tag: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let (mut i, mut code) = (0, false);
    while i < chars.len() {
        code ^= chars[i] == '`' || chars[i] == '$';
        let close = (chars[i] == marker && !code)
            .then(|| (i + 2..chars.len().min(i + 32)).find(|n| chars[*n] == marker))
            .flatten()
            .filter(|n| {
                chars[i + 1..*n]
                    .iter()
                    .all(|c| !c.is_whitespace() && *c != marker)
            });
        match close {
            Some(close) => {
                let inner: String = chars[i + 1..close].iter().collect();
                out.push_str(&format!("<{tag}>{inner}</{tag}>"));
                i = close + 1;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Highlights, tracked changes and raised text, which Markdown doesn't know, as the HTML
/// they mean. The paper reads the same tags.
pub fn marked(markdown: &str) -> String {
    let pairs = [
        ("{++", "<ins>"),
        ("++}", "</ins>"),
        ("{--", "<del>"),
        ("--}", "</del>"),
        ("{==", "<mark>"),
        ("==}", "</mark>"),
        ("{~~", "<del>"),
        ("~>", "</del><ins>"),
        ("~~}", "</ins>"),
        ("{>>", "<span class=\"note\">"),
        ("<<}", "</span>"),
    ];
    let mut out = String::with_capacity(markdown.len());
    let mut code = false;
    for line in markdown.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            code = !code;
        }
        if code {
            out.push_str(line);
            continue;
        }
        let mut text = line.to_string();
        for (from, to) in pairs {
            text = text.replace(from, to);
        }
        let text = raised(&raised(&text, '~', "sub"), '^', "sup");
        let mut parts = text.split("==");
        out.push_str(parts.next().unwrap_or(""));
        for (index, part) in parts.enumerate() {
            out.push_str(if index % 2 == 0 { "<mark>" } else { "</mark>" });
            out.push_str(part);
        }
    }
    out
}

/// The body of a Markdown document. Formulas that won't parse stay as their source.
pub fn body(markdown: &str) -> String {
    body_with(markdown, &|_| None)
}

/// What's already been built for a block of LaTeX, as HTML.
pub type Built<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// The body of a Markdown document, with what's been built for its blocks of TikZ.
pub fn body_with(markdown: &str, built: &Built) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
        | Options::ENABLE_MATH;
    let sized = figure::rewrite_sized(markdown, |image, width| {
        format!(
            "<img src=\"{}\" alt=\"{}\" style=\"width:{width}%\">",
            escape(image.path),
            escape(image.alt)
        )
    });
    let sized = marked(&sized);
    let mut events = Vec::new();
    let mut open: Option<(diagram::Kind, String)> = None;
    let mut tikz: Option<(String, String)> = None;
    for event in Parser::new_ext(&sized, options) {
        if let Some((language, source)) = &mut tikz {
            match event {
                Event::Text(text) => source.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    let html = built_or_source(language, source.trim_end(), built);
                    events.push(Event::Html(html.into()));
                    tikz = None;
                }
                _ => {}
            }
            continue;
        }
        match (&mut open, event) {
            (None, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))))
                if diagram::is_tikz(&language) =>
            {
                tikz = Some((language.to_string(), String::new()));
            }
            (None, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language))))
                if diagram::kind_of(&language).is_some() =>
            {
                open = diagram::kind_of(&language).map(|kind| (kind, String::new()));
            }
            (Some((_, source)), Event::Text(text)) => source.push_str(&text),
            (Some((kind, source)), Event::End(TagEnd::CodeBlock)) => {
                events.push(Event::Html(drawn(*kind, source).into()));
                open = None;
            }
            (_, Event::InlineMath(source)) => {
                events.push(Event::InlineHtml(formula(&source, false).into()));
            }
            (_, Event::DisplayMath(source)) => {
                events.push(Event::Html(formula(&source, true).into()));
            }
            (_, other) => events.push(other),
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// A whole page. `base` is the folder relative links and images start from.
pub fn page(title: &str, markdown: &str, base: &str) -> String {
    page_with(title, markdown, base, &|_| None)
}

/// A whole page, with what's been built for its blocks of TikZ.
pub fn page_with(title: &str, markdown: &str, base: &str, built: &Built) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <title>{}</title><base href=\"file://{}/\"><style>{STYLE}</style></head>\n<body>\n{}</body></html>\n",
        escape(title),
        escape(base),
        body_with(markdown, built)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_carries_tables_tasks_and_math() {
        let text = "# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n\nInline $x^2$ here.\n\n$$\\frac{1}{2}$$\n";
        let html = body(text);
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<table>"));
        assert!(html.contains("checkbox"));
        assert_eq!(html.matches("<svg").count(), 2, "{html}");
        assert!(html.contains("class=\"math\""));
    }

    #[test]
    fn diagrams_and_sized_images_reach_the_web_page() {
        let text = "```mermaid\nflowchart LR\n  A --> B\n```\n\n![Plot](a.png){width=40%}\n";
        let html = body(text);
        assert!(html.contains("class=\"diagram\""), "{html}");
        assert!(html.contains("<svg"), "{html}");
        assert!(!html.contains("flowchart"), "{html}");
        assert!(html.contains("style=\"width:40%\""), "{html}");
    }

    #[test]
    fn writing_marks_reach_the_web_page() {
        let html = body(
            "==key== H~2~O x^2^ {++new++}{--old--}[^a]\n\n[^a]: Note.\n\n> [!WARNING]\n> Careful.\n",
        );
        for want in [
            "<mark>key</mark>",
            "<sub>2</sub>",
            "<sup>2</sup>",
            "<ins>new</ins>",
            "<del>old</del>",
            "markdown-alert-warning",
            "Note.",
        ] {
            assert!(html.contains(want), "{want}: {html}");
        }
    }

    #[test]
    fn tikz_reaches_the_web_page_once_it_has_been_built() {
        let text = "```tikz\n\\draw (0,0);\n```\n";
        assert!(
            body(text).contains("\\draw"),
            "unbuilt, it stays as its source"
        );
        let html = body_with(text, &|tex| {
            assert!(tex.contains("\\begin{tikzpicture}"));
            Some("<img src=\"drawn\">".to_string())
        });
        assert!(html.contains("<img src=\"drawn\">"), "{html}");
        assert!(!html.contains("\\draw"), "{html}");
    }

    #[test]
    fn a_broken_diagram_stays_as_its_source() {
        let html = body("```mermaid\nnot a diagram (\n```\n");
        assert!(html.contains("not a diagram"), "{html}");
    }

    #[test]
    fn a_broken_formula_stays_as_its_source() {
        assert!(body("A $\\frac{$ b").contains("\\frac{"));
    }

    #[test]
    fn a_page_needs_no_network() {
        let html = page("Doc <1>", "text", "/tmp/here");
        assert!(html.contains("<title>Doc &lt;1&gt;</title>"));
        assert!(html.contains("file:///tmp/here/"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("http"));
    }
}
