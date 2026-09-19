//! SVG output (milestones 3 and 6).
//!
//! Turns a resolved [`Diagram`] plus a [`Layout`] into a self-contained,
//! static SVG document: rounded-rect boxes or cylinders with centered labels,
//! edges as direct (non-orthogonal) polylines carrying an auto-oriented
//! arrowhead, and — as of M6 — per-node `color`/`fill`, per-edge `color`,
//! the `dotted`/`dashed`/`thick` line styles, the `cylinder` shape, and edge
//! labels placed at the midpoint of each edge. The document is GitHub-
//! renderable — no scripts, no external references, no CSS dependencies, only
//! inline attributes.
//!
//! One arrowhead `<marker>` is emitted per distinct edge color, each with a
//! hard-coded `fill`, so a colored edge's arrowhead matches its line. This is
//! more portable than relying on `currentColor`/`context-stroke` inheriting
//! from the referencing element, which SVG markers don't do reliably across
//! renderers (including GitHub's). Edge labels are centered on the
//! edge's arc-length midpoint with a white knockout rect behind them, so the
//! line reads as broken behind the text (the classic Graphviz look).

use crate::ast::{Diagram, Edge, Node, Shape, Style};
use crate::layout::{EdgePath, Layout, NodeRect, SubgraphRect};
use crate::text;

/// Default stroke color for box borders and edges.
const STROKE: &str = "#333";
/// Default text color.
const INK: &str = "#222";
/// Default box interior fill (covers edges routed behind a node).
const FILL: &str = "#fff";
/// Box corner radius, in user units.
const RADIUS: f32 = 6.0;
/// Default edge stroke width, in user units.
const EDGE_WIDTH: f32 = 1.5;
/// Heavier stroke width for `thick` edges.
const THICK_WIDTH: f32 = 3.0;
/// Optical vertical offset (downward) for a cylinder's label, in user units.
/// A cylinder's top edge (the lid's front arc) and bottom edge (the base's
/// front arc) both bow downward in the middle, so a label centered on the
/// box's geometric center sits closer to the top edge than the bottom — the
/// shape reads as top-heavy. Shifting the label down by this amount recenters
/// it on the cylinder's *visual* middle. Pixel-measured against a rendered
/// screenshot: the 'd' in "Redis" is 19px tall and the label looks best 6px
/// lower, i.e. 6/19 of the font height. Boxes keep the geometric center.
const CYL_LABEL_SHIFT: f32 = (6.0 / 19.0) * crate::layout::FONT_SIZE;
/// Font size for edge labels (slightly smaller than the node label size).
const EDGE_LABEL_SIZE: f32 = 12.0;
/// Padding inside an edge label's white knockout rect, each side.
const LABEL_PAD: f32 = 3.0;
/// Fill of the knockout rect behind edge labels. Matches the default box fill
/// so the edge line is "broken" cleanly behind the text on a white page.
const LABEL_KNOCKOUT: &str = "#fff";

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
/// `diagram` supplies labels/shapes/styles/colors; `layout` supplies the
/// geometry produced by [`crate::layout::layout`]. The two are assumed to
/// correspond by index (nodes and edges in declaration order) — which the
/// layout engine guarantees.
pub fn render_svg(diagram: &Diagram, layout: &Layout) -> String {
    let mut s = String::new();
    let w = fmt(layout.width.max(1.0));
    let h = fmt(layout.height.max(1.0));

    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
    ));
    // One arrowhead marker per distinct edge color, each with a hard-coded
    // fill, so a colored edge's arrowhead matches its line. (SVG markers do
    // not reliably inherit `color`/stroke from the referencing element, so
    // `currentColor`/`context-stroke` can't be relied on across renderers —
    // notably GitHub's sanitizer.) The default-stroke marker is always
    // defined (`id="arrow"`) even with no edges, keeping output stable.
    let edge_color_ids = collect_edge_color_ids(&diagram.edges);
    s.push_str(&defs(&edge_color_ids));

    // Subgraph frames go behind edges (so a cross-boundary edge pierces the
    // frame border cleanly) and behind the nodes they contain. Omitted
    // entirely when there are no subgraphs, keeping subgraph-free output
    // byte-identical to the pre-M4 renderer.
    if !layout.subgraphs.is_empty() {
        s.push_str(&render_subgraphs(&layout.subgraphs));
    }

    // Edges first. Stroke color, width, and dash pattern are per-edge (M6:
    // line styles and edge colors); the shared group only sets no-fill and
    // the round caps/joins (round caps turn `dotted` into clean round dots).
    s.push_str("  <g fill=\"none\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n");
    for (edge, path) in diagram.edges.iter().zip(layout.edges.iter()) {
        let c = edge.color.clone().unwrap_or_else(|| STROKE.to_string());
        let mid = edge_color_ids
            .iter()
            .find(|(col, _)| col == &c)
            .map(|(_, id)| id.as_str())
            .unwrap_or("arrow");
        render_edge(&mut s, edge, path, mid);
    }
    s.push_str("  </g>\n");

    // Edge labels sit on top of the edge lines (a white knockout rect breaks
    // the line behind the text) but below the nodes, so a label that strays
    // over a node is covered by the node's fill.
    let labeled: Vec<(&Edge, &EdgePath)> = diagram
        .edges
        .iter()
        .zip(layout.edges.iter())
        .filter(|(e, _)| e.label.is_some())
        .collect();
    if !labeled.is_empty() {
        render_edge_labels(&mut s, &labeled);
    }

    // Nodes on top, so box fills hide any edge segments routed behind them.
    s.push_str("  <g>\n");
    for (node, rect) in diagram.nodes.iter().zip(layout.nodes.iter()) {
        render_node(&mut s, node, rect);
    }
    s.push_str("  </g>\n");

    s.push_str("</svg>\n");
    s
}

/// The `<defs>` block: one arrowhead `<marker>` per distinct edge color, each
/// with a hard-coded `fill` matching that color's line. The default-stroke
/// marker always has `id="arrow"`; custom colors get `id="arrow-<sanitized>"`
/// (see [`marker_id_for`]). This is more portable than relying on
/// `currentColor`/`context-stroke` inheriting from the referencing element,
/// which SVG markers don't do reliably across renderers (including GitHub's).
fn defs(edge_color_ids: &[(String, String)]) -> String {
    let mut s = String::from("  <defs>\n");
    for (color, id) in edge_color_ids {
        let color = escape_xml(color);
        s.push_str(&format!(
            "    <marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\"\n\
             \x20           markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\"\n\
             \x20           markerUnits=\"userSpaceOnUse\">\n\
             \x20     <path d=\"M0,0 L10,5 L0,10 Z\" fill=\"{color}\"/>\n\
             \x20   </marker>\n"
        ));
    }
    s.push_str("  </defs>\n");
    s
}

/// The distinct edge colors, in declaration order, each mapped to a unique
/// marker id. The default stroke color ([`STROKE`]) is always present and
/// maps to `"arrow"`; custom colors map to `"arrow-<sanitized>"` (see
/// [`marker_id_for`]).
fn collect_edge_color_ids(edges: &[Edge]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![(STROKE.to_string(), "arrow".to_string())];
    for edge in edges {
        let c = edge.color.clone().unwrap_or_else(|| STROKE.to_string());
        if !out.iter().any(|(col, _)| col == &c) {
            let id = marker_id_for(&c, &out);
            out.push((c, id));
        }
    }
    out
}

/// A unique marker id for `color`: `"arrow"` for the default stroke, else
/// `"arrow-"` plus the lowercase alphanumeric chars of the color (so
/// `"#888"` → `"arrow-888"`, `"#0a7"` → `"arrow-0a7"`). If that name
/// already exists, a `-2`, `-3`, … suffix is appended until it's unique, so
/// two distinct colors that sanitize identically never collide.
fn marker_id_for(color: &str, existing: &[(String, String)]) -> String {
    if color == STROKE {
        return "arrow".to_string();
    }
    let base: String = "arrow-".to_string()
        + &color
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>();
    let taken = |name: &str| -> bool {
        name == "arrow" || existing.iter().any(|(_, id)| id == name)
    };
    let mut id = base.clone();
    let mut n = 2;
    while taken(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
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
/// Per-edge stroke color, width, and dash pattern carry the M6 line styles
/// (`dotted`/`dashed`/`thick`) and the optional `color` attribute. The
/// arrowhead is the per-color marker `marker_id` (one marker per distinct
/// edge color is emitted in `<defs>`), so a colored edge's arrowhead matches
/// its line.
fn render_edge(s: &mut String, edge: &Edge, path: &EdgePath, marker_id: &str) {
    if path.points.is_empty() {
        return;
    }
    let color = escape_xml(edge.color.as_deref().unwrap_or(STROKE));
    let (width, dash) = edge_stroke(edge.style);
    let mut pts = String::new();
    for (i, (x, y)) in path.points.iter().enumerate() {
        if i > 0 {
            pts.push(' ');
        }
        pts.push_str(&fmt(*x));
        pts.push(',');
        pts.push_str(&fmt(*y));
    }
    s.push_str(&format!(
        "    <polyline points=\"{pts}\" stroke=\"{color}\" stroke-width=\"{}\"",
        width,
    ));
    if let Some(d) = dash {
        s.push_str(&format!(" stroke-dasharray=\"{d}\""));
    }
    s.push_str(&format!(" marker-end=\"url(#{marker_id})\"/>\n"));
}

/// Map an edge [`Style`] to its (stroke-width, optional dash pattern).
fn edge_stroke(style: Style) -> (f32, Option<&'static str>) {
    match style {
        Style::Solid => (EDGE_WIDTH, None),
        // A 1-on dash with round caps renders as a row of round dots.
        Style::Dotted => (EDGE_WIDTH, Some("1 4")),
        Style::Dashed => (EDGE_WIDTH, Some("6 4")),
        Style::Thick => (THICK_WIDTH, None),
    }
}

/// Edge labels: each is centered on the midpoint of its polyline (by arc
/// length), with a white knockout rect behind the text so the edge line reads
/// as broken behind the label. Drawn after the edges and before the nodes.
fn render_edge_labels(s: &mut String, labeled: &[(&Edge, &EdgePath)]) {
    // Knockout rects first (their own group), then the text (another group),
    // so no rect can cover a sibling label's text.
    s.push_str(&format!("  <g fill=\"{LABEL_KNOCKOUT}\" stroke=\"none\">\n"));
    for (edge, path) in labeled {
        let label = edge.label.as_deref().unwrap();
        let (mx, my) = point_at_fraction(&path.points, 0.5);
        let m = text::measure(label, EDGE_LABEL_SIZE);
        let rw = m.width + 2.0 * LABEL_PAD;
        let rh = m.height + 2.0 * LABEL_PAD;
        s.push_str(&format!(
            "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>\n",
            fmt(mx - rw / 2.0),
            fmt(my - rh / 2.0),
            fmt(rw),
            fmt(rh),
        ));
    }
    s.push_str("  </g>\n");
    s.push_str(&format!(
        "  <g fill=\"{INK}\" font-family=\"DejaVu Sans, Arial, sans-serif\" font-size=\"{}\" text-anchor=\"middle\" dominant-baseline=\"central\">\n",
        fmt(EDGE_LABEL_SIZE)
    ));
    for (edge, path) in labeled {
        let label = edge.label.as_deref().unwrap();
        let (mx, my) = point_at_fraction(&path.points, 0.5);
        s.push_str(&format!(
            "    <text x=\"{}\" y=\"{}\">{}</text>\n",
            fmt(mx),
            fmt(my),
            escape_xml(label),
        ));
    }
    s.push_str("  </g>\n");
}

/// The point at `frac` (0..1) of the arc length along a polyline. Used to
/// place edge labels at the visual midpoint of their edge.
fn point_at_fraction(points: &[(f32, f32)], frac: f32) -> (f32, f32) {
    if points.is_empty() {
        return (0.0, 0.0);
    }
    if points.len() == 1 {
        return points[0];
    }
    let mut total = 0.0_f32;
    let mut lens: Vec<f32> = Vec::with_capacity(points.len() - 1);
    for w in points.windows(2) {
        let (ax, ay) = w[0];
        let (bx, by) = w[1];
        let l = (bx - ax).hypot(by - ay).max(1e-6);
        lens.push(l);
        total += l;
    }
    let target = total * frac;
    let mut acc = 0.0_f32;
    for (i, &l) in lens.iter().enumerate() {
        let prev = acc;
        acc += l;
        if acc >= target || i == lens.len() - 1 {
            let (ax, ay) = points[i];
            let (bx, by) = points[i + 1];
            let t = if l > 1e-6 {
                ((target - prev) / l).clamp(0.0, 1.0)
            } else {
                0.0
            };
            return (ax + t * (bx - ax), ay + t * (by - ay));
        }
    }
    *points.last().unwrap()
}

/// One node: a rounded rect (or cylinder) plus a centered label. The node's
/// `color` (stroke) and `fill` (interior) attributes, and the `cylinder`
/// shape, are honored (M6).
fn render_node(s: &mut String, node: &Node, rect: &NodeRect) {
    let stroke = escape_xml(node.color.as_deref().unwrap_or(STROKE));
    let fill = escape_xml(node.fill.as_deref().unwrap_or(FILL));
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    match node.shape {
        Shape::Box => {
            s.push_str(&format!(
                "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{EDGE_WIDTH}\"/>\n",
                fmt(rect.x),
                fmt(rect.y),
                fmt(rect.w),
                fmt(rect.h),
                RADIUS,
                RADIUS,
            ));
        }
        Shape::Cylinder => {
            render_cylinder(s, rect, &stroke, &fill);
        }
    }
    // A cylinder's lid and base both bow downward in the middle, so the box's
    // geometric center looks too high for the label; shift it down by
    // [`CYL_LABEL_SHIFT`] to land on the visual middle. Boxes keep the
    // geometric center.
    let label_y = match node.shape {
        Shape::Cylinder => cy + CYL_LABEL_SHIFT,
        Shape::Box => cy,
    };
    s.push_str(&format!(
        "    <text x=\"{}\" y=\"{}\" font-family=\"DejaVu Sans, Arial, sans-serif\" font-size=\"{}\" fill=\"{INK}\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>\n",
        fmt(cx),
        fmt(label_y),
        crate::layout::FONT_SIZE,
        escape_xml(&node.label),
    ));
}

/// A cylinder: a body path (two vertical sides, a front-bottom arc, and a
/// back-top arc) plus a full top ellipse for the lid. The cap radius is the
/// fixed [`crate::layout::CYL_RY`], set in the layout so a cylinder's height
/// already includes room for the caps plus breathing space for the label.
/// The label is drawn by [`render_node`], which shifts it down from the box
/// center by [`CYL_LABEL_SHIFT`] so it sits on the cylinder's visual middle
/// (both caps bow downward, so the geometric center reads as too high).
///
/// Geometry: the top ellipse is centered at `y + ry` and the bottom at
/// `y + h - ry`, so the caps fit exactly inside the laid-out box rect. The
/// body's straight sides run between the two ellipse centers; the front-bottom
/// arc bulges **downward** (the visible front rim of the bottom, its midpoint
/// the lowest point of the shape) and the back-top arc bulges upward, hidden
/// behind the lid ellipse drawn on top of it.
fn render_cylinder(s: &mut String, rect: &NodeRect, stroke: &str, fill: &str) {
    let ry = crate::layout::CYL_RY;
    let rx = rect.w / 2.0;
    let cx = rect.x + rect.w / 2.0;
    let y_top = rect.y + ry;
    let y_bot = rect.y + rect.h - ry;
    let left = cx - rx;
    let right = cx + rx;
    // Body: down the left side, front-bottom arc (sweep 0 on a left→right
    // chord ⇒ bulges downward in SVG's y-down space, the front rim of the
    // bottom), up the right side, back-top arc (sweep 0 on a right→left chord
    // ⇒ bulges upward, hidden by the lid), close. The two arcs share sweep=0
    // but traverse in opposite directions, giving opposite bulges.
    s.push_str(&format!(
        "    <path d=\"M{},{} L{},{} A{},{} 0 0 0 {},{} L{},{} A{},{} 0 0 0 {},{} Z\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{EDGE_WIDTH}\"/>\n",
        fmt(left),
        fmt(y_top),
        fmt(left),
        fmt(y_bot),
        fmt(rx),
        fmt(ry),
        fmt(right),
        fmt(y_bot),
        fmt(right),
        fmt(y_top),
        fmt(rx),
        fmt(ry),
        fmt(left),
        fmt(y_top),
        fill,
        stroke,
    ));
    // Lid: full top ellipse, drawn over the body's back-top arc.
    s.push_str(&format!(
        "    <ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{EDGE_WIDTH}\"/>\n",
        fmt(cx),
        fmt(y_top),
        fmt(rx),
        fmt(ry),
        fill,
        stroke,
    ));
}

/// Format a float with two decimal places. Deterministic across runs and
/// machines, which is what the snapshot tests rely on.
fn fmt(v: f32) -> String {
    format!("{:.2}", v)
}

/// Escape the five XML-significant characters for safe inclusion in text
/// content and attribute values. Labels and color values are user-supplied,
/// so this is required even for v0.
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

    // ---- M6: shapes, styles, colors, edge labels ----

    #[test]
    fn cylinder_renders_lid_ellipse_and_body_path() {
        let svg = render("diagram top-down\ndb \"Pg\" : cylinder\n");
        // The lid is an <ellipse>; the body is a <path> with two arcs.
        assert!(svg.contains("<ellipse "), "missing cylinder lid ellipse");
        assert!(svg.contains("<path d=\"M"), "missing cylinder body path");
        // A single cylinder node (no edges, no subgraphs) emits no <rect>.
        assert!(!svg.contains("<rect"), "cylinder node should not emit a <rect>");
    }

    #[test]
    fn cylinder_bottom_curves_downward() {
        // The front-bottom arc must bulge DOWNWARD — its midpoint is the
        // lowest point of the shape (the front rim of the bottom), not an
        // upward frown into the body. In SVG (y-down), sweep-flag 0 on a
        // left→right chord bulges down; sweep-flag 1 bulges up (the bug this
        // guards against).
        let raw = parser::parse_diagram("diagram top-down\ndb \"Pg\" : cylinder\n").unwrap();
        let d = resolve::resolve(&raw).unwrap();
        let l = layout::layout(&d);
        let svg = render_svg(&d, &l);
        let rect = l.nodes.iter().find(|n| n.id == "db").unwrap();
        let body = svg
            .lines()
            .find(|l| l.contains("<path d=\"M") && l.contains("A"))
            .unwrap();
        assert!(
            !body.contains("0 0 1"),
            "cylinder body has an upward-bulging arc (sweep 1); the front-bottom must sweep 0 (downward):\n{body}"
        );
        // The front-bottom arc runs left→right at y_bot with sweep 0, so its
        // midpoint (y_bot + ry) sits exactly on the box rect's bottom edge.
        let ry = crate::layout::CYL_RY;
        let rx = rect.w / 2.0;
        let right = rect.x + rect.w;
        let y_bot = rect.y + rect.h - ry;
        let arc = format!("A{:.2},{:.2} 0 0 0 {:.2},{:.2}", rx, ry, right, y_bot);
        assert!(
            body.contains(&arc),
            "front-bottom arc `{arc}` not in body path:\n{body}"
        );
    }

    #[test]
    fn cylinder_label_clears_lid_with_breathing_room() {
        // The label is optically shifted down from the box center (see
        // [`CYL_LABEL_SHIFT`]) so it sits on the cylinder's visual middle.
        // It must still clear the lid's bottom curve above and stay within
        // the straight-sided body below (above the seam where the sides meet
        // the base arc). With `dominant-baseline="central"` and DejaVu Sans
        // (ascent+descent == em height), the glyph extent equals the em box,
        // centered on the shifted `label_y`.
        let raw = parser::parse_diagram(
            "diagram top-down\n\u{64}b \"Postgres\" : cylinder\n",
        )
        .unwrap();
        let d = resolve::resolve(&raw).unwrap();
        let l = layout::layout(&d);
        let svg = render_svg(&d, &l);
        let rect = l.nodes.iter().find(|n| n.id == "db").unwrap();
        let ry = crate::layout::CYL_RY;
        let em = crate::layout::FONT_SIZE;
        let shift = (6.0 / 19.0) * crate::layout::FONT_SIZE;
        let cy = rect.y + rect.h / 2.0;
        let label_y = cy + shift;
        let glyph_top = label_y - em / 2.0;
        let glyph_bottom = label_y + em / 2.0;
        let lid_bottom = rect.y + 2.0 * ry; // lowest point of the lid ellipse
        let base_seam = rect.y + rect.h - ry; // where the straight sides meet the base arc
        let top_clear = glyph_top - lid_bottom;
        let bottom_clear = base_seam - glyph_bottom;
        assert!(
            top_clear >= 3.0,
            "label collides with the lid: glyph_top={glyph_top:.2} lid_bottom={lid_bottom:.2} clear={top_clear:.2}"
        );
        assert!(
            bottom_clear >= 3.0,
            "label dips below the base seam: glyph_bottom={glyph_bottom:.2} base_seam={base_seam:.2} clear={bottom_clear:.2}"
        );
        // The label <text> is drawn at the optically shifted y (not the box center).
        let text = svg
            .lines()
            .find(|l| l.contains("<text") && l.contains(">Postgres</text>"))
            .unwrap();
        assert!(
            text.contains(&format!("y=\"{}\"", fmt(label_y))),
            "label not at shifted y={label_y:.2} (cy={cy:.2}): {text}"
        );
    }

    #[test]
    fn cylinder_label_is_optically_lowered() {
        // A cylinder's label is shifted down from the box center by 6/19 of
        // the font height (optical centering: the lid and base both bow
        // down in the middle, so the geometric center looks too high). A
        // box's label stays at the geometric center. Lock both so the
        // behavior can't regress silently.
        let raw =
            parser::parse_diagram("diagram top-down\nb \"Box\"\nc \"Cyl\" : cylinder\n").unwrap();
        let d = resolve::resolve(&raw).unwrap();
        let l = layout::layout(&d);
        let svg = render_svg(&d, &l);
        let box_rect = l.nodes.iter().find(|n| n.id == "b").unwrap();
        let cyl_rect = l.nodes.iter().find(|n| n.id == "c").unwrap();
        let box_cy = box_rect.y + box_rect.h / 2.0;
        let cyl_cy = cyl_rect.y + cyl_rect.h / 2.0;
        let shift = (6.0 / 19.0) * crate::layout::FONT_SIZE;
        assert!(shift > 0.0);
        let box_text = svg.lines().find(|l| l.contains(">Box</text>")).unwrap();
        let cyl_text = svg.lines().find(|l| l.contains(">Cyl</text>")).unwrap();
        assert!(
            box_text.contains(&format!("y=\"{}\"", fmt(box_cy))),
            "box label should be at the geometric center (no shift): {box_text}"
        );
        assert!(
            !box_text.contains(&format!("y=\"{}\"", fmt(box_cy + shift))),
            "box label should NOT be shifted: {box_text}"
        );
        assert!(
            cyl_text.contains(&format!("y=\"{}\"", fmt(cyl_cy + shift))),
            "cylinder label should be shifted down by {shift:.3} to {}: {cyl_text}",
            fmt(cyl_cy + shift)
        );
    }

    #[test]
    fn box_node_still_renders_a_rect() {
        let svg = render("diagram top-down\na \"A\"\n");
        assert!(svg.contains("<rect "));
        assert!(!svg.contains("<ellipse "));
    }

    #[test]
    fn node_color_and_fill_are_rendered() {
        let svg = render("diagram top-down\na \"A\" color=\"#888\" fill=\"#eef\"\n");
        let box_line = svg.lines().find(|l| l.contains("<rect")).unwrap();
        assert!(box_line.contains("fill=\"#eef\""), "custom fill not on box: {box_line}");
        assert!(box_line.contains("stroke=\"#888\""), "custom stroke not on box: {box_line}");
    }

    #[test]
    fn edge_color_renders_on_stroke_and_arrow() {
        // A colored edge's line AND arrowhead match its color. The arrowhead
        // is a per-color marker (id `arrow-<sanitized>`) with a hard-coded
        // fill, since SVG markers don't reliably inherit `color`/stroke from
        // the referencing element. The default-stroke marker stays `arrow`.
        let svg = render("diagram top-down\na -- color=\"#888\" --> b\n");
        let line = svg.lines().find(|l| l.contains("<polyline")).unwrap();
        assert!(line.contains("stroke=\"#888\""), "edge color not on stroke: {line}");
        // The edge references its own colored marker, not the default `arrow`.
        assert!(
            line.contains("marker-end=\"url(#arrow-888)\""),
            "colored edge should reference `arrow-888`, got: {line}"
        );
        // The colored marker exists with a matching hard-coded fill (the
        // `<path fill>` lives on its own line inside the marker).
        assert!(
            svg.contains("<marker id=\"arrow-888\""),
            "no `arrow-888` marker in:\n{svg}"
        );
        assert!(
            svg.contains("fill=\"#888\""),
            "colored marker should be filled #888\n{svg}"
        );
        // And the default `arrow` marker (default stroke) is still defined.
        assert!(svg.contains("<marker id=\"arrow\""), "default `arrow` marker missing");
        // No reliance on currentColor anywhere.
        assert!(!svg.contains("currentColor"), "renderer must not emit currentColor");
    }

    #[test]
    fn default_edge_uses_default_arrow_marker() {
        // An edge with no custom color references the default `arrow` marker
        // (filled with the default stroke), and no custom markers are emitted.
        let svg = render("diagram top-down\na --> b\n");
        let line = svg.lines().find(|l| l.contains("<polyline")).unwrap();
        assert!(line.contains("marker-end=\"url(#arrow)\""));
        // The default marker's path is filled with the default stroke (#333).
        assert!(svg.contains("fill=\"#333\""), "default marker fill should be #333\n{svg}");
        // Only the one default marker is defined.
        assert_eq!(svg.matches("<marker").count(), 1);
    }

    #[test]
    fn two_distinct_edge_colors_get_distinct_markers() {
        // Two different custom colors get two distinct markers, each filled
        // with its color; the default `arrow` marker is still present.
        let svg = render("diagram top-down\na -- color=\"#888\" --> b\nb -- color=\"#0a7\" --> c\n");
        assert!(svg.contains("<marker id=\"arrow\""));
        assert!(svg.contains("<marker id=\"arrow-888\""));
        assert!(svg.contains("<marker id=\"arrow-0a7\""));
        assert!(svg.contains("fill=\"#888\""));
        assert!(svg.contains("fill=\"#0a7\""));
        assert_eq!(svg.matches("<marker").count(), 3);
    }

    #[test]
    fn same_color_edges_share_one_marker() {
        // Two edges with the same custom color share a single colored marker.
        let svg = render("diagram top-down\na -- color=\"#888\" --> b\nc -- color=\"#888\" --> d\n");
        // default `arrow` + one `arrow-888` only.
        assert_eq!(svg.matches("<marker").count(), 2);
        let e1 = svg.lines().filter(|l| l.contains("<polyline")).nth(0).unwrap();
        let e2 = svg.lines().filter(|l| l.contains("<polyline")).nth(1).unwrap();
        assert!(e1.contains("url(#arrow-888)"));
        assert!(e2.contains("url(#arrow-888)"));
    }

    #[test]
    fn dotted_edge_has_dot_dasharray() {
        let svg = render("diagram top-down\na -- dotted --> b\n");
        assert!(svg.contains("stroke-dasharray=\"1 4\""));
        // A solid edge has no dasharray.
        let svg2 = render("diagram top-down\na --> b\n");
        assert!(!svg2.contains("stroke-dasharray"));
    }

    #[test]
    fn dashed_edge_has_dash_dasharray() {
        let svg = render("diagram top-down\na -- dashed --> b\n");
        assert!(svg.contains("stroke-dasharray=\"6 4\""));
    }

    #[test]
    fn thick_edge_has_wider_stroke() {
        let svg = render("diagram top-down\na -- thick --> b\n");
        let line = svg.lines().find(|l| l.contains("<polyline")).unwrap();
        assert!(line.contains("stroke-width=\"3\""), "thick edge not wider: {line}");
        // Thick is still solid (no dasharray).
        assert!(!line.contains("stroke-dasharray"));
    }

    #[test]
    fn edge_label_is_rendered_with_knockout() {
        let svg = render("diagram top-down\na -- \"sync\" --> b\n");
        assert!(svg.contains(">sync</text>"), "edge label text missing");
        // The knockout is a dedicated white, stroke-less group.
        assert!(svg.contains("<g fill=\"#fff\" stroke=\"none\">"));
    }

    #[test]
    fn edge_label_is_xml_escaped() {
        let svg = render("diagram top-down\na -- \"x<y&z>\" --> b\n");
        assert!(svg.contains("&lt;y&amp;z&gt;"));
        assert!(!svg.contains("<y&z>"));
    }

    #[test]
    fn unlabeled_edge_emits_no_label_group() {
        let svg = render("diagram top-down\na --> b\n");
        assert!(!svg.contains("<g fill=\"#fff\" stroke=\"none\">"));
    }

    #[test]
    fn point_at_fraction_endpoints() {
        assert_eq!(point_at_fraction(&[], 0.5), (0.0, 0.0));
        assert_eq!(point_at_fraction(&[(1.0, 2.0)], 0.5), (1.0, 2.0));
        // Midpoint of a single segment.
        assert_eq!(point_at_fraction(&[(0.0, 0.0), (10.0, 0.0)], 0.5), (5.0, 0.0));
        // Halfway along a two-segment path of equal length lands at the joint.
        assert_eq!(point_at_fraction(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 0.5), (10.0, 0.0));
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

    #[test]
    fn snapshot_shapes_styles() {
        // The full M6 surface in one diagram: cylinder nodes, node color/fill,
        // edge styles (dotted/dashed/thick), edge color, and edge labels.
        assert_snapshot(
            "shapes_styles",
            &render(
                "diagram top-down\n\
                 src \"Source\"\n\
                 db \"Postgres\" : cylinder color=\"#888\" fill=\"#eef\"\n\
                 cache \"Redis\" : cylinder\n\
                 sink \"Sink\" color=\"#0a7\" fill=\"#cfe\"\n\
                 src -- dotted \"polls\" --> db\n\
                 src -- dashed color=\"#888\" --> cache\n\
                 db -- thick \"replication\" color=\"#888\" --> sink\n",
            ),
        );
    }
}
