//! A Markdown file as a web page that stands alone: math is inline SVG, so the page needs
//! no scripts and no network.

use pulldown_cmark::{Event, Options, Parser, html};

use crate::math;

const STYLE: &str = "
:root { color-scheme: light dark; --text: #1f2328; --dim: #59636e; --line: #d1d9e0;
  --code: #f6f8fa; --link: #0969da; }
@media (prefers-color-scheme: dark) { :root { --text: #e6edf3; --dim: #9198a1;
  --line: #3d444d; --code: #151b23; --link: #4493f8; } svg { filter: invert(1); } }
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
        shown
    }
}

/// The body of a Markdown document. Formulas that won't parse stay as their source.
pub fn body(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::InlineMath(source) => Event::InlineHtml(formula(&source, false).into()),
        Event::DisplayMath(source) => Event::Html(formula(&source, true).into()),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
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
