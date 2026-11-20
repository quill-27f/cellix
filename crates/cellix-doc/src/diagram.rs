//! Diagrams written as text, drawn by pure Rust so a page needs no browser or Node. Each kind
//! comes out as SVG, which a web page keeps and a page in the editor turns into pixels.

use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Once, OnceLock};

use layout::backends::svg::SVGWriter;
use layout::gv::{DotParser, GraphBuilder};
use mermaid_rs_renderer::{
    DiagramKind, Layout, RenderOptions, Theme, compute_layout, parse_mermaid_strict, render_svg,
};
use resvg::usvg;

/// What a fenced block is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mermaid,
    Graphviz,
    /// SVG written straight into the document.
    Svg,
}

/// Whether a fence holds TikZ, which only LaTeX can draw.
pub fn is_tikz(language: &str) -> bool {
    matches!(language.split_whitespace().next(), Some("tikz" | "tikzcd"))
}

/// A fenced block of TikZ as LaTeX, in its environment unless it brought its own.
pub fn fenced_tikz(language: &str, source: &str) -> String {
    let name = if language.starts_with("tikzcd") {
        "tikzcd"
    } else {
        "tikzpicture"
    };
    if source.contains("\\begin{") {
        source.to_string()
    } else {
        format!("\\begin{{{name}}}\n{source}\n\\end{{{name}}}")
    }
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

/// What a new drawing in a paper starts as: its name and its LaTeX.
pub const PAPER_TEMPLATES: [(&str, &str); 4] = [
    (
        "Commutative Diagram",
        "\\begin{tikzcd}\n  A \\arrow[r, \"f\"] \\arrow[d, \"g\"'] & B \\arrow[d, \"h\"] \\\\\n  C \\arrow[r, \"k\"'] & D\n\\end{tikzcd}",
    ),
    (
        "Flow",
        "\\begin{tikzpicture}[every node/.style={draw, rounded corners, inner sep=6pt}]\n  \\node (a) at (0,0) {Input};\n  \\node (b) at (3,0) {Model};\n  \\node (c) at (6,0) {Output};\n  \\draw[->] (a) -- (b);\n  \\draw[->] (b) -- (c);\n\\end{tikzpicture}",
    ),
    (
        "Axes and Curve",
        "\\begin{tikzpicture}\n  \\draw[->] (0,0) -- (4,0) node[right] {$x$};\n  \\draw[->] (0,0) -- (0,3) node[above] {$y$};\n  \\draw[thick] (0,0) .. controls (1,3) and (3,0) .. (4,2.5);\n\\end{tikzpicture}",
    ),
    (
        "Tree",
        "\\begin{tikzpicture}[level distance=12mm, sibling distance=20mm]\n  \\node {Root}\n    child { node {Left} }\n    child { node {Right} };\n\\end{tikzpicture}",
    ),
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

/// A pie's slices, and their swatches, in colours that stand 3:1 or more from the page and
/// apart from each other, on a light sheet and a dark one. The theme's own were pale greys and
/// blues, the first white on white beside "Writing" (67-sample-charts).
const PIE_LIGHT: [&str; 12] = [
    "#3f6fd8", "#21877b", "#b5720c", "#c8456a", "#7a5ad0", "#5a6a80", "#3f8f3f", "#c8642a",
    "#2a86a8", "#b04a92", "#7e7e2a", "#8a5a3a",
];
const PIE_DARK: [&str; 12] = [
    "#7aa2f7", "#4fd1c5", "#f2c26b", "#f28fad", "#b49cf5", "#94a3b8", "#7fcf7f", "#f5a26b",
    "#6cc8e6", "#e08fd0", "#c8c86b", "#c89a70",
];

fn mermaid(source: &str, dark: bool) -> Result<String, String> {
    let options = mermaid_options(dark);
    let layout = laid_out(source, &options)?;
    Ok(render_svg(&layout, &options.theme, &options.layout))
}

/// A Mermaid diagram laid out as the renderer lays it, and then what Cellix changes of that.
fn laid_out(source: &str, options: &RenderOptions) -> Result<Layout, String> {
    let parsed = parse_mermaid_strict(source).map_err(|e| first_line(&e.to_string()))?;
    let mut layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
    labels_beside_short_edges(&mut layout);
    Ok(layout)
}

/// Units between an edge and a label stood beside it.
const LABEL_BESIDE: f32 = 8.0;
/// What the renderer draws a state diagram's edge label with: its words at 0.85 of the
/// theme's size, and 3 units either side of them.
const STATE_LABEL_SCALE: f32 = 0.85;
const STATE_LABEL_PAD: f32 = 3.0;

/// A state diagram's labelled edge to a state right under it: the renderer leaves the gap
/// between them the label's height and routes the edge round its own label, out to the side
/// and back, so its arrowhead stood on the label's foot ("approved", 66). Such an edge goes
/// straight down, as Mermaid draws it, and its label stands beside it, in the middle of the
/// gap, 8 units clear of the line. The picture is made wider where the label needs it.
fn labels_beside_short_edges(layout: &mut Layout) {
    if layout.kind != DiagramKind::State {
        return;
    }
    let mut width = layout.width;
    for edge in &mut layout.edges {
        let (Some(label), Some((_, at))) = (&edge.label, edge.label_anchor) else {
            continue;
        };
        let (Some(&(x0, y0)), Some(&(x1, y1))) = (edge.points.first(), edge.points.last()) else {
            continue;
        };
        let down = (x0 - x1).abs() < 1.0 && y1 > y0;
        let round = edge.points.iter().any(|&(x, _)| (x - x0).abs() >= 1.0);
        if !down || !round || !(y0..y1).contains(&at) {
            continue;
        }
        edge.points = vec![(x0, y0), (x1, y1)];
        let half = label.width * STATE_LABEL_SCALE / 2.0 + STATE_LABEL_PAD;
        let middle = x0 + LABEL_BESIDE + half;
        edge.label_anchor = Some((middle, (y0 + y1) / 2.0));
        width = width.max(middle + half + LABEL_BESIDE);
    }
    layout.width = width;
}

fn mermaid_options(dark: bool) -> RenderOptions {
    let mut theme = if dark { Theme::dark() } else { Theme::modern() };
    // The page shows through, in whichever theme it has.
    theme.background = "transparent".to_string();
    let (slices, words) = if dark {
        (PIE_DARK, "#17191d")
    } else {
        (PIE_LIGHT, "#ffffff")
    };
    theme.pie_colors = slices.map(String::from);
    theme.pie_section_text_color = words.to_string();
    // Whole, as their swatches are: at the theme's 0.7 to 0.85 the slices were paler than the
    // swatches beside their names, (91,131,221) for "Writing"'s (63,111,216), and nearer the
    // page (0.12.0's release review, finding 11).
    theme.pie_opacity = 1.0;
    RenderOptions {
        theme,
        ..RenderOptions::default()
    }
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

    /// The sample's state diagram (samples/writing.md, 66).
    const STATES: &str = "stateDiagram-v2\n  [*] --> Draft\n  Draft --> Review\n  \
                          Review --> Draft : changes wanted\n  Review --> [*] : approved\n";

    /// A labelled edge to the state right under it goes straight down with its label beside
    /// it, 8 units clear of the line, in the middle of the gap: the renderer routed it round
    /// its own label and ended it under the label's foot, so its arrowhead stood on the
    /// label's border ("approved", 66). An edge that goes round for its own sake is left be.
    #[test]
    fn a_labelled_edge_straight_down_has_its_label_beside_it() {
        let labelled = |layout: &Layout, words: &str| {
            let found = layout.edges.iter().find(|edge| {
                edge.label
                    .as_ref()
                    .is_some_and(|label| label.lines.iter().any(|line| line == words))
            });
            found
                .cloned()
                .unwrap_or_else(|| panic!("no edge says {words}"))
        };
        let options = mermaid_options(false);
        let parsed = parse_mermaid_strict(STATES).unwrap();
        let routed = compute_layout(&parsed.graph, &options.theme, &options.layout);
        assert!(
            labelled(&routed, "approved").points.len() > 2,
            "the renderer goes round the label: the case this is for"
        );
        let layout = laid_out(STATES, &options).unwrap();
        let edge = labelled(&layout, "approved");
        assert_eq!(edge.points.len(), 2, "{:?}", edge.points);
        let ((x0, y0), (x1, y1)) = (edge.points[0], edge.points[1]);
        assert!((x0 - x1).abs() < 1.0 && y1 > y0, "{:?}", edge.points);
        // The label's box as the renderer draws it.
        let label = edge.label.as_ref().unwrap();
        let (middle, at) = edge.label_anchor.unwrap();
        let half = label.width * STATE_LABEL_SCALE / 2.0 + STATE_LABEL_PAD;
        assert!(
            (middle - half - x0 - LABEL_BESIDE).abs() < 0.01,
            "8 units from the line"
        );
        assert!(
            (at - (y0 + y1) / 2.0).abs() < 0.01,
            "in the middle of the gap"
        );
        assert!(
            middle + half + LABEL_BESIDE <= layout.width + 0.01,
            "inside the picture"
        );
        // The edge back up goes round the states, as it did.
        let back = labelled(&layout, "changes wanted");
        assert_eq!(back.points, labelled(&routed, "changes wanted").points);
        // And it's drawn: the label in the picture, which is wider for it.
        let svg = mermaid(STATES, false).unwrap();
        assert!(svg.contains("approved"));
        assert!(inked(&rasterize(&svg, 1.0, 900).unwrap()) > 50);
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

    /// The contrast of two colours written `#rrggbb`, as WCAG reckons it.
    fn contrast(a: &str, b: &str) -> f32 {
        let luminance = |hex: &str| {
            let channel = |at: usize| {
                let c = f32::from(u8::from_str_radix(&hex[at..at + 2], 16).unwrap()) / 255.0;
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// A pie's slices and their swatches stand 3:1 or more from the page in both themes, and
    /// the slices' numbers 3:1 from them: the theme's first colour was near white, its swatch
    /// white on white beside "Writing" (0.11.0's final review, S6, 67-sample-charts).
    #[test]
    fn a_pies_colours_stand_apart_from_the_page() {
        let source = "pie title Share\n  \"A\" : 40\n  \"B\" : 35\n  \"C\" : 25\n";
        for (dark, page, words, slices) in [
            (false, "#ffffff", "#ffffff", PIE_LIGHT),
            (true, "#17191d", "#17191d", PIE_DARK),
        ] {
            let svg = svg(Kind::Mermaid, source, dark).unwrap().to_lowercase();
            for colour in &slices[..3] {
                assert!(svg.contains(colour), "{colour} drawn, dark: {dark}");
            }
            for colour in slices {
                assert!(contrast(colour, page) >= 3.0, "{colour} on {page}");
                assert!(contrast(colour, words) >= 3.0, "{words} on {colour}");
            }
        }
    }

    /// A pie's slices are drawn in their colours whole, as the swatches beside their names are,
    /// so each slice is its swatch's colour on the page: at the theme's opacity, "Writing"'s
    /// slice was (91,131,221) beside its swatch's (63,111,216) (0.12.0's release review, finding
    /// 11, 67-sample-charts).
    #[test]
    fn a_pies_slices_are_their_swatches_colours() {
        let source = "pie title Share\n  \"A\" : 40\n  \"B\" : 35\n  \"C\" : 25\n";
        for (dark, slices) in [(false, PIE_LIGHT), (true, PIE_DARK)] {
            let svg = svg(Kind::Mermaid, source, dark).unwrap();
            let picture = rasterize(&svg, 2.0, 1600).unwrap();
            for colour in &slices[..3] {
                let hex = |at: usize| u8::from_str_radix(&colour[at..at + 2], 16).unwrap();
                let whole = [hex(1), hex(3), hex(5), 255];
                let count = (picture.rgba.chunks(4)).filter(|p| *p == whole).count();
                // A slice's thousands of pixels, not a swatch's few hundred.
                assert!(
                    count > 5000,
                    "{colour} whole in {count} pixels, dark: {dark}"
                );
            }
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
