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

/// A diagram as a picture, or its source if it can't be drawn.
fn drawn(kind: diagram::Kind, source: &str) -> String {
    match diagram::svg(kind, source, false) {
        Ok(svg) => format!("<div class=\"diagram\">{svg}</div>"),
        Err(_) => format!("<pre><code>{}</code></pre>", escape(source)),
    }
}

/// The body of a Markdown document. Formulas that won't parse stay as their source.
pub fn body(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;
    let sized = figure::rewrite_sized(markdown, |image, width| {
        format!(
            "<img src=\"{}\" alt=\"{}\" style=\"width:{width}%\">",
            escape(image.path),
            escape(image.alt)
        )
    });
    let mut events = Vec::new();
    let mut open: Option<(diagram::Kind, String)> = None;
    for event in Parser::new_ext(&sized, options) {
        match (&mut open, event) {
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
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <title>{}</title><base href=\"file://{}/\"><style>{STYLE}</style></head>\n<body>\n{}</body></html>\n",
        escape(title),
        escape(base),
        body(markdown)
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
