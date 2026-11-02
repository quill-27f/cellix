//! Diagrams written as text, drawn by pure Rust so a page needs no browser or Node. Each kind
//! comes out as SVG, which a web page keeps and a page in the editor turns into pixels.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Once, OnceLock};

use layout::backends::svg::SVGWriter;
use layout::gv::{DotParser, GraphBuilder};
use mermaid_rs_renderer::{RenderOptions, Theme, render_with_options};
use resvg::usvg;

/// What a fenced block is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mermaid,
    Graphviz,
    /// SVG written straight into the document.
    Svg,
}

/// The kind a fence's language names.
pub fn kind_of(language: &str) -> Option<Kind> {
    match language.split_whitespace().next()? {
        "mermaid" | "mmd" => Some(Kind::Mermaid),
        "dot" | "graphviz" => Some(Kind::Graphviz),
        "svg" => Some(Kind::Svg),
        _ => None,
    }
}

/// What a new diagram starts as: its name, its fence's language and its source.
pub const TEMPLATES: [(&str, &str, &str); 9] = [
    (
        "Flowchart",
        "mermaid",
        "flowchart LR\n  A[Start] --> B{Decide}\n  B -->|Yes| C[Do it]\n  B -->|No| D[Stop]",
    ),
    (
        "Sequence",
        "mermaid",
        "sequenceDiagram\n  You->>Agent: Ask\n  Agent-->>You: Answer",
    ),
    (
        "Class",
        "mermaid",
        "classDiagram\n  class Model {\n    +forward(x)\n  }\n  Model <|-- Transformer",
    ),
    (
        "State",
        "mermaid",
        "stateDiagram-v2\n  [*] --> Draft\n  Draft --> Review\n  Review --> [*]",
    ),
    (
        "Entity Relationship",
        "mermaid",
        "erDiagram\n  AUTHOR ||--o{ PAPER : writes",
    ),
    (
        "Gantt",
        "mermaid",
        "gantt\n  title Plan\n  section Work\n  Draft :a1, 2026-01-01, 14d\n  Review :after a1, 7d",
    ),
    (
        "Pie",
        "mermaid",
        "pie title Share\n  \"A\" : 60\n  \"B\" : 40",
    ),
    (
        "Mind Map",
        "mermaid",
        "mindmap\n  root((Topic))\n    First\n    Second",
    ),
    ("Graph", "dot", "digraph {\n  a -> b\n  b -> c\n}"),
];

thread_local! {
    /// Set while a diagram is being drawn, when a half-typed one may panic the library.
    static DRAWING: Cell<bool> = const { Cell::new(false) };
}

/// Runs `draw`, turning a panic into an error. Text being typed is often not a diagram yet,
/// and a message printed over the screen is as bad as a crash.
fn guarded<T>(draw: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    static QUIET: Once = Once::new();
    QUIET.call_once(|| {
        let shown = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !DRAWING.with(Cell::get) {
                shown(info);
            }
        }));
    });
    DRAWING.with(|d| d.set(true));
    let result = panic::catch_unwind(AssertUnwindSafe(draw));
    DRAWING.with(|d| d.set(false));
    result.unwrap_or_else(|_| Err("This diagram can't be drawn yet".to_string()))
}

fn mermaid(source: &str, dark: bool) -> Result<String, String> {
    let mut theme = if dark { Theme::dark() } else { Theme::modern() };
    // The page shows through, in whichever theme it has.
    theme.background = "transparent".to_string();
    let options = RenderOptions {
        theme,
        ..RenderOptions::default()
    };
    render_with_options(source, options).map_err(|e| first_line(&e.to_string()))
}

fn graphviz(source: &str, dark: bool) -> Result<String, String> {
    let graph = DotParser::new(source)
        .process()
        .map_err(|e| first_line(&e))?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    let mut writer = SVGWriter::new();
    visual.do_it(false, false, false, &mut writer);
    let svg = writer.finalize();
    // The layout draws black on white, so the dark sheet gets the opposite.
    Ok(if dark {
        svg.replace("black", "#cdd6f4").replace("white", "none")
    } else {
        svg.replace("white", "none")
    })
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).to_string()
}

/// Draws a diagram as SVG.
pub fn svg(kind: Kind, source: &str, dark: bool) -> Result<String, String> {
    match kind {
        Kind::Mermaid => guarded(|| mermaid(source, dark)),
        Kind::Graphviz => guarded(|| graphviz(source, dark)),
        Kind::Svg => Ok(source.to_string()),
    }
}

/// Pixels ready for the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Straight, not premultiplied.
    pub rgba: Vec<u8>,
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

/// Turns SVG into pixels at `scale`, or smaller if that would pass `max_width`.
pub fn rasterize(svg: &str, scale: f32, max_width: u32) -> Result<Raster, String> {
    guarded(|| {
        let options = usvg::Options {
            fontdb: fonts(),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(svg, &options).map_err(|e| first_line(&e.to_string()))?;
        let size = tree.size();
        let fit = max_width as f32 / size.width().max(1.0);
        let scale = scale.min(fit).max(0.01);
        let width = (size.width() * scale).ceil().max(1.0) as u32;
        let height = (size.height() * scale).ceil().max(1.0) as u32;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "This picture is too big".to_string())?;
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let mut rgba = Vec::with_capacity(pixmap.data().len());
        for pixel in pixmap.pixels() {
            let pixel = pixel.demultiply();
            rgba.extend([pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
        }
        Ok(Raster {
            width,
            height,
            rgba,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOW: &str = "flowchart LR\n  A[Start] --> B{Choice}\n  B -->|Yes| C[Done]\n";

    fn inked(raster: &Raster) -> usize {
        raster.rgba.chunks(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn every_new_diagram_draws() {
        for (name, language, source) in TEMPLATES {
            let kind = kind_of(language).unwrap();
            let svg = svg(kind, source, false).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(inked(&rasterize(&svg, 1.0, 900).unwrap()) > 50, "{name}");
        }
    }

    #[test]
    fn fences_name_their_kind() {
        assert_eq!(kind_of("mermaid"), Some(Kind::Mermaid));
        assert_eq!(kind_of("dot"), Some(Kind::Graphviz));
        assert_eq!(kind_of("graphviz"), Some(Kind::Graphviz));
        assert_eq!(kind_of("rust"), None);
        assert_eq!(kind_of(""), None);
    }

    #[test]
    fn every_common_mermaid_diagram_draws() {
        for source in [
            FLOW,
            "sequenceDiagram\n  Alice->>Bob: Hello\n  Bob-->>Alice: Hi\n",
            "classDiagram\n  class Model {\n    +forward()\n  }\n  Model <|-- Net\n",
            "stateDiagram-v2\n  [*] --> Train\n  Train --> [*]\n",
            "erDiagram\n  USER ||--o{ ORDER : places\n",
            "gantt\n  title Plan\n  section A\n  Task :a1, 2024-01-01, 30d\n",
            "pie title Share\n  \"A\" : 40\n  \"B\" : 60\n",
            "mindmap\n  root((Paper))\n    Method\n    Results\n",
        ] {
            let svg = svg(Kind::Mermaid, source, false).unwrap();
            let picture = rasterize(&svg, 2.0, 1600).unwrap();
            assert!(inked(&picture) > 100, "{source}");
        }
    }

    #[test]
    fn dark_and_light_differ() {
        assert_ne!(
            svg(Kind::Mermaid, FLOW, true).unwrap(),
            svg(Kind::Mermaid, FLOW, false).unwrap()
        );
    }

    #[test]
    fn graphviz_draws() {
        let svg = svg(Kind::Graphviz, "digraph { a -> b; b -> c; }", false).unwrap();
        assert!(inked(&rasterize(&svg, 2.0, 800).unwrap()) > 50);
    }

    #[test]
    fn half_typed_diagrams_are_errors_and_never_panics() {
        for source in [
            FLOW,
            "sequenceDiagram\n  Alice->>Bob: Hello\n",
            "digraph { a -> b [label=\"x\"]; }",
            "pie title Share\n  \"A\" : 40\n",
        ] {
            let kind = if source.starts_with("digraph") {
                Kind::Graphviz
            } else {
                Kind::Mermaid
            };
            for end in 0..source.len() {
                let _ = svg(kind, &source[..end], false);
            }
        }
        assert!(svg(Kind::Mermaid, "not a diagram at all (", false).is_err());
    }

    #[test]
    fn a_wide_picture_is_kept_to_the_page() {
        let svg = svg(Kind::Mermaid, FLOW, false).unwrap();
        let picture = rasterize(&svg, 4.0, 200).unwrap();
        assert!(picture.width <= 200, "{}", picture.width);
    }

    #[test]
    fn plain_svg_becomes_pixels() {
        let text = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'>\
                    <rect width='10' height='10' fill='#ff0000'/></svg>";
        let picture = rasterize(text, 2.0, 100).unwrap();
        assert_eq!((picture.width, picture.height), (20, 20));
        assert_eq!(&picture.rgba[..4], &[255, 0, 0, 255]);
    }
}
