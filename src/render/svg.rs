//! SVG output (milestone 3).
//!
//! Turns a resolved [`Diagram`] plus a [`Layout`] into a self-contained,
//! static SVG document: rounded-rect boxes with centered labels, and edges as
//! direct (non-orthogonal) polylines carrying an auto-oriented arrowhead via
//! a `<marker>`. The document is GitHub-renderable — no scripts, no external
//! references, no CSS dependencies, only inline attributes.
//!
//! Scope for v0/M3: every node is drawn as a rounded box (the `cylinder`
//! shape and per-edge styles/colors/labels are parsed but rendered in later
//! milestones). The structure here is deliberately factored so those land as
//! small additions.

use crate::ast::{Diagram, Edge, Node};
use crate::layout::{EdgePath, Layout, NodeRect, SubgraphRect};

/// Default stroke color for box borders and edges.
const STROKE: &str = "#333";
/// Default text color.
const INK: &str = "#222";
/// Default box interior fill (covers edges routed behind a node).
const FILL: &str = "#fff";
/// Box corner radius, in user units.
const RADIUS: f32 = 6.0;
/// Edge stroke width, in user units.
const EDGE_WIDTH: f32 = 1.5;

// Subgraph frame styling. Frames are deliberately lighter and thinner than
// node boxes so the contained nodes read as the foreground.
const FRAME_STROKE: &str = "#7a7a7a";
const FRAME_STROKE_WIDTH: f32 = 1.0;
const FRAME_RADIUS: f32 = 8.0;
const FRAME_TITLE_FILL: &str = "#3a3a3a";
const FRAME_TITLE_SIZE: f32 = 12.0;
/// Inset of the title text from the frame's top-left corner.
const FRAME_TITLE_X: f32 = 10.0;
const FRAME_TITLE_Y: f32 = 4.0;

/// Render a laid-out diagram to a self-contained SVG string.
///
/// `diagram` supplies labels/shapes/styles; `layout` supplies the geometry
/// produced by [`crate::layout::layout`]. The two are assumed to correspond
/// by index (nodes and edges in declaration order) — which the layout engine
/// guarantees.
pub fn render_svg(diagram: &Diagram, layout: &Layout) -> String {
    let mut s = String::new();
    let w = fmt(layout.width.max(1.0));
    let h = fmt(layout.height.max(1.0));

    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
    ));
    s.push_str(&defs());

    // Subgraph frames go behind edges (so a cross-boundary edge pierces the
    // frame border cleanly) and behind the nodes they contain. Omitted
    // entirely when there are no subgraphs, keeping subgraph-free output
    // byte-identical to the pre-M4 renderer.
    if !layout.subgraphs.is_empty() {
        s.push_str(&render_subgraphs(&layout.subgraphs));
    }

    // Edges, then nodes on top, so box fills hide any edge segments
    // routed behind them.
    s.push_str(&format!(
        "  <g fill=\"none\" stroke=\"{STROKE}\" stroke-width=\"{EDGE_WIDTH}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n"
    ));
    for (edge, path) in diagram.edges.iter().zip(layout.edges.iter()) {
        render_edge(&mut s, edge, path);
    }
    s.push_str("  </g>\n");

    s.push_str("  <g>\n");
    for (node, rect) in diagram.nodes.iter().zip(layout.nodes.iter()) {
        render_node(&mut s, node, rect);
    }
    s.push_str("  </g>\n");

    s.push_str("</svg>\n");
    s
}

/// The `<defs>` block: a single arrowhead marker reused by every edge.
fn defs() -> String {
    format!(
        "  <defs>\n\
         \x20   <marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\"\n\
         \x20           markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\"\n\
         \x20           markerUnits=\"userSpaceOnUse\">\n\
         \x20     <path d=\"M0,0 L10,5 L0,10 Z\" fill=\"{STROKE}\"/>\n\
         \x20   </marker>\n\
         \x20 </defs>\n"
    )
}

/// The subgraph frames: one rounded, transparent rectangle per subgraph
/// with its title set into the top-left of the frame. Drawn before edges and
/// nodes so contained boxes and crossing edges render on top of the border.
fn render_subgraphs(subgraphs: &[SubgraphRect]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "  <g fill=\"none\" stroke=\"{FRAME_STROKE}\" stroke-width=\"{FRAME_STROKE_WIDTH}\" stroke-linejoin=\"round\">\n"
    ));
    for sg in subgraphs {
        s.push_str(&format!(
            "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"none\" stroke=\"{FRAME_STROKE}\" stroke-width=\"{FRAME_STROKE_WIDTH}\"/>\n",
            fmt(sg.x),
            fmt(sg.y),
            fmt(sg.w),
            fmt(sg.h),
            FRAME_RADIUS,
            FRAME_RADIUS,
        ));
    }
    s.push_str("  </g>\n");
    // Titles ride on top of the frame border.
    s.push_str(&format!(
        "  <g fill=\"{FRAME_TITLE_FILL}\" font-family=\"DejaVu Sans, Arial, sans-serif\" font-size=\"{}\">\n",
        fmt(FRAME_TITLE_SIZE)
    ));
    for sg in subgraphs {
        if let Some(title) = &sg.title {
            s.push_str(&format!(
                "    <text x=\"{}\" y=\"{}\" text-anchor=\"start\" dominant-baseline=\"hanging\">{}</text>\n",
                fmt(sg.x + FRAME_TITLE_X),
                fmt(sg.y + FRAME_TITLE_Y),
                escape_xml(title),
            ));
        }
    }
    s.push_str("  </g>\n");
    s
}

/// One edge as a polyline through its waypoints, with an arrowhead at the end.
fn render_edge(s: &mut String, _edge: &Edge, path: &EdgePath) {
    if path.points.is_empty() {
        return;
    }
    s.push_str("    <polyline points=\"");
    for (i, (x, y)) in path.points.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&fmt(*x));
        s.push(',');
        s.push_str(&fmt(*y));
    }
    s.push_str("\" marker-end=\"url(#arrow)\"/>\n");
}

/// One node: a rounded rect plus a centered label.
fn render_node(s: &mut String, node: &Node, rect: &NodeRect) {
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    s.push_str(&format!(
        "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{FILL}\" stroke=\"{STROKE}\" stroke-width=\"{EDGE_WIDTH}\"/>\n",
        fmt(rect.x),
        fmt(rect.y),
        fmt(rect.w),
        fmt(rect.h),
        RADIUS,
        RADIUS,
    ));
    s.push_str(&format!(
        "    <text x=\"{}\" y=\"{}\" font-family=\"DejaVu Sans, Arial, sans-serif\" font-size=\"{}\" fill=\"{INK}\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>\n",
        fmt(cx),
        fmt(cy),
        crate::layout::FONT_SIZE,
        escape_xml(&node.label),
    ));
}

/// Format a float with two decimal places. Deterministic across runs and
/// machines, which is what the snapshot tests rely on.
fn fmt(v: f32) -> String {
    format!("{:.2}", v)
}

/// Escape the five XML-significant characters for safe inclusion in text
/// content and attribute values. Labels are user-supplied, so this is
/// required even for v0.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;
    use crate::parser;
    use crate::resolve;
    use std::path::PathBuf;

    /// Directory holding the golden `.svg` files.
    fn snapshot_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots")
    }

    /// Parse, resolve, lay out, and render a source diagram to SVG.
    fn render(src: &str) -> String {
        let raw = parser::parse_diagram(src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let l = layout::layout(&d);
        render_svg(&d, &l)
    }

    /// Compare `actual` SVG against the golden file `name.svg`. When the
    /// `UPDATE_SNAPSHOTS` environment variable is set, (re)write the golden
    /// file instead of comparing — this is how the harness is kept up to date.
    fn assert_snapshot(name: &str, actual: &str) {
        let dir = snapshot_dir();
        let path = dir.join(format!("{name}.svg"));
        if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(&dir).expect("create snapshot dir");
            std::fs::write(&path, actual).expect("write golden file");
            return;
        }
        let expected = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => panic!(
                "missing snapshot {path:?}: {e}\n\
                 run `UPDATE_SNAPSHOTS=1 cargo test` to create it"
            ),
        };
        assert_eq!(
            actual, expected,
            "snapshot mismatch for `{name}`;\n\
             run `UPDATE_SNAPSHOTS=1 cargo test` to regenerate the golden file"
        );
    }

    // ---- Unit checks (not golden) ----

    #[test]
    fn escape_covers_all_special_chars() {
        assert_eq!(escape_xml("a<b>&\"'c"), "a&lt;b&gt;&amp;&quot;&#39;c");
        assert_eq!(escape_xml("plain text 123"), "plain text 123");
    }

    #[test]
    fn empty_diagram_is_valid_svg() {
        let svg = render("diagram top-down\n");
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        // No nodes or edges, just the (def-less-content) structure.
        assert!(!svg.contains("<rect"));
        assert!(!svg.contains("<polyline"));
    }

    #[test]
    fn single_node_has_rect_and_label() {
        let svg = render("diagram top-down\nweb \"Web Server\"\n");
        assert!(svg.contains("<rect "));
        assert!(svg.contains("dominant-baseline=\"central\""));
        // Label is XML-escaped into the text element.
        assert!(svg.contains(">Web Server</text>"));
        // Arrowhead marker is always defined.
        assert!(svg.contains("<marker id=\"arrow\""));
    }

    #[test]
    fn edge_has_polyline_and_marker_end() {
        let svg = render("diagram top-down\na-->b\n");
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("marker-end=\"url(#arrow)\""));
    }

    #[test]
    fn label_with_xml_chars_is_escaped() {
        let svg = render("diagram top-down\na \"x<y&z>\"\n");
        assert!(svg.contains("&lt;y&amp;z&gt;"));
        assert!(!svg.contains("<y&z>"));
    }

    #[test]
    fn edges_drawn_before_nodes() {
        // The edges <g> must open before the nodes <g> so boxes cover edge
        // stubs routed behind them.
        let svg = render("diagram top-down\na-->b\n");
        let edges = svg.find("<g fill=\"none\"").unwrap();
        let nodes = svg.find("  <g>\n    <rect").unwrap();
        assert!(edges < nodes);
    }

    #[test]
    fn svg_has_viewbox_sized_to_content() {
        let svg = render("diagram top-down\na-->b-->c-->d\n");
        // viewBox present and finite, matching width/height attributes.
        assert!(svg.contains("viewBox=\""));
        let w = svg
            .find("width=\"")
            .map(|i| &svg[i..])
            .and_then(|s| s.split('"').nth(1))
            .unwrap();
        assert!(svg.contains(&format!("viewBox=\"0 0 {w} ")));
    }

    // ---- Golden snapshots (lock the output) ----

    #[test]
    fn snapshot_simple_chain() {
        assert_snapshot("simple_chain", &render("diagram top-down\na-->b-->c\n"));
    }

    #[test]
    fn snapshot_left_right_chain() {
        assert_snapshot(
            "left_right_chain",
            &render("diagram left-right\na-->b-->c\n"),
        );
    }

    #[test]
    fn snapshot_diamond() {
        assert_snapshot(
            "diamond",
            &render("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n"),
        );
    }

    #[test]
    fn snapshot_cycle() {
        assert_snapshot("cycle", &render("diagram top-down\na-->b\nb-->a\n"));
    }

    #[test]
    fn snapshot_isolated_nodes() {
        assert_snapshot("isolated_nodes", &render("diagram top-down\na\nb\nc\n"));
    }

    #[test]
    fn snapshot_infra() {
        assert_snapshot("infra", &render(include_str!("../../examples/infra.mmd")));
    }

    #[test]
    fn snapshot_subdirection() {
        // The headline M4 feature: two subgraphs with different per-subgraph
        // directions (one left-right, one top-down) in a single top-down
        // diagram, plus cross-boundary edges.
        assert_snapshot(
            "subdirection",
            &render(include_str!("../../examples/subdirection.mmd")),
        );
    }
}
