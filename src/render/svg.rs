//! SVG output (milestones 3, 6, 7, 7.5, and 13).
//!
//! Turns a resolved [`Diagram`] plus a [`Layout`] into a self-contained,
//! static SVG document: rounded-rect boxes or cylinders with centered labels,
//! edges as orthogonal (right-angle) polylines carrying an auto-oriented
//! arrowhead, and — as of M6 — per-node `color`/`fill`, per-edge `color`,
//! the `dotted`/`dashed`/`thick` line styles, the `cylinder` shape, and edge
//! labels. M7.5 adds
//! per-subgraph frame `color`/`fill`/`line` (border style) and a `text`
//! text-color attribute on node labels, edge labels, and subgraph titles.
//! The document is GitHub-
//! renderable — no scripts, no external references, no CSS dependencies, only
//! inline attributes.
//!
//! The orthogonal edge geometry (M7) is produced by the layout engine; the
//! renderer just strokes each edge's waypoint polyline as given. The shared
//! `orient="auto"` arrowhead orients itself along the (now axis-aligned)
//! final segment, so the arrow points straight at the target with no
//! per-edge marker work.
//!
//! One arrowhead `<marker>` is emitted per distinct edge color, each with a
//! hard-coded `fill`, so a colored edge's arrowhead matches its line. This is
//! more portable than relying on `currentColor`/`context-stroke` inheriting
//! from the referencing element, which SVG markers don't do reliably across
//! renderers (including GitHub's). Edge labels are drawn at the anchor the
//! layout's label placement engine picks (M13, stored per-edge on
//! [`EdgePath::label_at`]) with a white knockout rect behind them, so the line
//! reads as broken behind the text (the classic Graphviz look). The renderer
//! only draws: the placement policy — candidates along the polyline scored
//! against the finished geometry — lives in `layout.rs`.
//!
//! Each edge's line is shortened at the target end by [`ARROW_BACKOFF`] so the
//! stroke tucks under its same-color arrowhead: a thick line's round cap
//! would otherwise ride past the arrowhead's tip and read as a blunt, square
//! arrow. The marker's `refX` is reduced by the same amount so the tip still
//! lands on the target node's boundary.

use crate::ast::{Diagram, Edge, Node, Shape, Style, Subgraph};
use crate::layout::{
    EdgePath, Layout, NodeRect, SubgraphRect, EDGE_LABEL_SIZE, LABEL_PAD,
};
use crate::text;

/// Default stroke color for edges (and the default arrowhead fill).
const STROKE: &str = "#333";
/// Default node border color (when no `color` attribute is given).
const NODE_STROKE: &str = "#2196F3";
/// Default text color.
const INK: &str = "#222";
/// Default node interior fill (when no `fill` attribute is given). Covers
/// edges routed behind a node.
const FILL: &str = "#c0e1fc";
/// Box corner radius, in user units.
const RADIUS: f32 = 6.0;
/// Default edge stroke width, in user units.
const EDGE_WIDTH: f32 = 1.5;
/// Heavier stroke width for `thick` edges.
const THICK_WIDTH: f32 = 3.0;
/// How far an edge's line ends short of its arrowhead tip, so the stroke
/// tucks under the (same-color) arrowhead instead of poking past it. A thick
/// line's round cap is half the stroke wide and bulges that far past the line
/// end, so this is sized to clear the widest stroke (`THICK_WIDTH` plus its
/// cap) with margin; the same-color arrowhead fill makes the gap seamless for
/// thinner strokes. The arrowhead `<marker>`'s `refX` is reduced by this same
/// amount (see [`defs`]) so the tip still lands on the target node's
/// boundary.
const ARROW_BACKOFF: f32 = 4.0;
/// Optical vertical offset (downward) for a cylinder's label, in user units.
/// A cylinder's top edge (the lid's front arc) and bottom edge (the base's
/// front arc) both bow downward in the middle, so a label centered on the
/// box's geometric center sits closer to the top edge than the bottom — the
/// shape reads as top-heavy. Shifting the label down by this amount recenters
/// it on the cylinder's *visual* middle. Pixel-measured against a rendered
/// screenshot: the 'd' in "Redis" is 19px tall and the label looks best 6px
/// lower, i.e. 6/19 of the font height. Boxes keep the geometric center.
const CYL_LABEL_SHIFT: f32 = (6.0 / 19.0) * crate::layout::FONT_SIZE;
/// Font stack for every `<text>` element. Leads with the metric-compatible
/// trio (Arial/Helvetica/Liberation Sans are interchangeable clones) so
/// virtually every viewer renders identical glyph geometry; DejaVu Sans
/// stays last as the Linux catch-all. Layout *measures* with the wider
/// bundled DejaVu (`text.rs`), so boxes never under-size for any of these.
const FONT_FAMILY: &str = "Arial, Helvetica, Liberation Sans, DejaVu Sans, sans-serif";
// Edge-label sizing ([`EDGE_LABEL_SIZE`], [`LABEL_PAD`]) lives in
// `layout.rs`: the placement engine sizes the very knockout rects this module
// draws around its anchors (M13), so both sides share one definition.
/// Fill of the knockout rect behind edge labels. Matches the white page
/// background so the edge line is "broken" cleanly behind the text.
const LABEL_KNOCKOUT: &str = "#fff";

// Subgraph frame styling. Frames are deliberately lighter and thinner than
// node boxes so the contained nodes read as the foreground.
const FRAME_STROKE: &str = "#88BDA4";
/// Default frame interior fill (when no `fill` attribute is given).
const FRAME_FILL: &str = "#f2f8f4";
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
        s.push_str(&render_subgraphs(&diagram.subgraphs, &layout.subgraphs));
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
    // The marker's tip sits `refX` ahead of the line end. Reducing refX from
    // the tip (10) by [`ARROW_BACKOFF`] offsets the line shortening done in
    // [`render_edge`], so the arrowhead tip still lands on the target node's
    // boundary while the line tucks under it.
    let ref_x = fmt(10.0 - ARROW_BACKOFF);
    for (color, id) in edge_color_ids {
        let color = escape_xml(color);
        s.push_str(&format!(
            "    <marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"{ref_x}\" refY=\"5\"\n\
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

/// The subgraph frames: one rounded rectangle per subgraph with its title
/// set into the top-left of the frame. Drawn before edges and nodes so
/// contained boxes and crossing edges render on top of the border.
///
/// `subgraphs` (the resolved [`Subgraph`]s, carrying style) and `rects`
/// (the laid-out [`SubgraphRect`]s, carrying geometry) correspond by index
/// (declaration order), so they're zipped — the same pattern the renderer
/// uses for nodes and edges. A frame's `color` (border), `fill` (background),
/// `line` (border style), and `text` (title color) are honored (M7.5); a
/// frame with none of these falls back to the default frame palette
/// ([`FRAME_STROKE`] border, [`FRAME_FILL`] interior, the group's default
/// stroke-width, and a title color inherited from the group).
fn render_subgraphs(subgraphs: &[Subgraph], rects: &[SubgraphRect]) -> String {
    let mut s = String::new();
    // The group sets the default frame stroke/width (the M4 defaults); each
    // frame's <rect> overrides fill/stroke/width/dasharray with its own
    // `color`/`fill`/`line` (M7.5). A subgraph with no `fill` gets the
    // default frame fill ([`FRAME_FILL`]); an explicit `fill="none"` keeps
    // the frame transparent so edges routed behind it stay visible.
    s.push_str(&format!(
        "  <g fill=\"none\" stroke=\"{FRAME_STROKE}\" stroke-width=\"{FRAME_STROKE_WIDTH}\" stroke-linejoin=\"round\">\n"
    ));
    for (sg, r) in subgraphs.iter().zip(rects.iter()) {
        let stroke = escape_xml(sg.color.as_deref().unwrap_or(FRAME_STROKE));
        let fill = escape_xml(sg.fill.as_deref().unwrap_or(FRAME_FILL));
        let (width, dash) = frame_stroke(sg.line);
        s.push_str(&format!(
            "    <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{}\"",
            fmt(r.x),
            fmt(r.y),
            fmt(r.w),
            fmt(r.h),
            FRAME_RADIUS,
            FRAME_RADIUS,
            width,
        ));
        if let Some(d) = dash {
            s.push_str(&format!(" stroke-dasharray=\"{d}\""));
        }
        s.push_str("/>\n");
    }
    s.push_str("  </g>\n");
    // Titles ride on top of the frame border. The group sets the default
    // title color; a subgraph with a `text` attribute overrides it on its own
    // <text> so the default (no `text`) title stays byte-identical.
    s.push_str(&format!(
        "  <g fill=\"{FRAME_TITLE_FILL}\" font-family=\"{FONT_FAMILY}\" font-size=\"{}\">\n",
        fmt(FRAME_TITLE_SIZE)
    ));
    for (sg, r) in subgraphs.iter().zip(rects.iter()) {
        if let Some(title) = &sg.title {
            // Each line's top rides at its own `y` (hanging baseline): line
            // 0 at `FRAME_TITLE_Y`, one [`text::line_height`] per extra line
            // — matching the top inset [`frame_insets`] reserves.
            let n = text::line_count(title);
            let lh = text::line_height(FRAME_TITLE_SIZE);
            let ys: Vec<f32> = (0..n).map(|i| r.y + FRAME_TITLE_Y + i as f32 * lh).collect();
            let body = text_body(title, r.x + FRAME_TITLE_X, &ys);
            if let Some(c) = &sg.text {
                s.push_str(&format!(
                    "    <text x=\"{}\" y=\"{}\" fill=\"{}\" text-anchor=\"start\" dominant-baseline=\"hanging\">{}</text>\n",
                    fmt(r.x + FRAME_TITLE_X),
                    fmt(r.y + FRAME_TITLE_Y),
                    escape_xml(c),
                    body,
                ));
            } else {
                s.push_str(&format!(
                    "    <text x=\"{}\" y=\"{}\" text-anchor=\"start\" dominant-baseline=\"hanging\">{}</text>\n",
                    fmt(r.x + FRAME_TITLE_X),
                    fmt(r.y + FRAME_TITLE_Y),
                    body,
                ));
            }
        }
    }
    s.push_str("  </g>\n");
    s
}

/// Map a subgraph frame's optional `line` style (M7.5) to its
/// (stroke-width, optional dash pattern). `None` (or `solid`) is the default
/// plain frame; `dotted`/`dashed` reuse the same dash patterns as edges;
/// `thick` doubles the base frame stroke width (mirroring the edge
/// convention where `thick` doubles the base edge width). Widths are emitted
/// via plain `Display` (not [`fmt`]) so the default width renders as `"1"`,
/// byte-identical to the pre-M7.5 frame output.
fn frame_stroke(style: Option<Style>) -> (f32, Option<&'static str>) {
    match style {
        None | Some(Style::Solid) => (FRAME_STROKE_WIDTH, None),
        // A 1-on dash with round caps renders as a row of round dots.
        Some(Style::Dotted) => (FRAME_STROKE_WIDTH, Some("1 4")),
        Some(Style::Dashed) => (FRAME_STROKE_WIDTH, Some("6 4")),
        Some(Style::Thick) => (2.0 * FRAME_STROKE_WIDTH, None),
    }
}

/// One edge as a polyline through its waypoints, with an arrowhead at the end.
/// Per-edge stroke color, width, and dash pattern carry the M6 line styles
/// (`dotted`/`dashed`/`thick`) and the optional `color` attribute. The
/// arrowhead is the per-color marker `marker_id` (one marker per distinct
/// edge color is emitted in `<defs>`), so a colored edge's arrowhead matches
/// its line. The final waypoint is pulled back by [`ARROW_BACKOFF`] along the
/// last segment so the line ends under the arrowhead rather than poking past
/// its tip (most visible on `thick` edges); the marker's reduced `refX` keeps
/// the tip on the target node's boundary.
fn render_edge(s: &mut String, edge: &Edge, path: &EdgePath, marker_id: &str) {
    if path.points.is_empty() {
        return;
    }
    let color = escape_xml(edge.color.as_deref().unwrap_or(STROKE));
    let (width, dash) = edge_stroke(edge.style);
    // The line ends `ARROW_BACKOFF` short of the last waypoint so the stroke
    // tucks under the arrowhead (drawn at the original tip by the marker,
    // whose `refX` is reduced by the same amount) instead of poking past it.
    let points = shortened_points(path);
    let mut pts = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
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

/// The edge's waypoints with the final point pulled back by [`ARROW_BACKOFF`]
/// along the last segment, so the line ends under the arrowhead (which the
/// marker draws at the original tip) instead of poking past it. Clamped to the
/// last segment's length so a pathologically short final segment can't flip
/// backwards — with the layout's minimum edge segment (~8, a cross-boundary
/// frame fan) that clamp never triggers in practice, since `ARROW_BACKOFF` is
/// well below it.
fn shortened_points(path: &EdgePath) -> Vec<(f32, f32)> {
    let mut pts = path.points.clone();
    let n = pts.len();
    if n >= 2 {
        let (px, py) = pts[n - 2];
        let (lx, ly) = pts[n - 1];
        let dx = lx - px;
        let dy = ly - py;
        let len = dx.hypot(dy);
        if len > 1e-6 {
            let back = ARROW_BACKOFF.min(len - 1e-3).max(0.0);
            pts[n - 1].0 = lx - back * dx / len;
            pts[n - 1].1 = ly - back * dy / len;
        }
    }
    pts
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

/// Edge labels: each is drawn at the anchor the layout's label placement
/// engine picked (M13, [`EdgePath::label_at`] — the center of the knockout
/// rect + text block, chosen against the finished geometry), with a white
/// knockout rect behind the text so the edge line reads as broken behind the
/// label. Drawn after the edges and before the nodes, so a label that strays
/// over a node is covered by the node's fill.
fn render_edge_labels(s: &mut String, labeled: &[(&Edge, &EdgePath)]) {
    // Knockout rects first (their own group), then the text (another group),
    // so no rect can cover a sibling label's text.
    s.push_str(&format!("  <g fill=\"{LABEL_KNOCKOUT}\" stroke=\"none\">\n"));
    for (edge, path) in labeled {
        let label = edge.label.as_deref().unwrap();
        let (mx, my) = label_anchor(path);
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
        "  <g fill=\"{INK}\" font-family=\"{FONT_FAMILY}\" font-size=\"{}\" text-anchor=\"middle\" dominant-baseline=\"central\">\n",
        fmt(EDGE_LABEL_SIZE)
    ));
    for (edge, path) in labeled {
        let label = edge.label.as_deref().unwrap();
        let (mx, my) = label_anchor(path);
        let ys = centered_line_ys(my, text::line_count(label), EDGE_LABEL_SIZE);
        if let Some(c) = &edge.text {
            s.push_str(&format!(
                "    <text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>\n",
                fmt(mx),
                fmt(ys[0]),
                escape_xml(c),
                text_body(label, mx, &ys),
            ));
        } else {
            s.push_str(&format!(
                "    <text x=\"{}\" y=\"{}\">{}</text>\n",
                fmt(mx),
                fmt(ys[0]),
                text_body(label, mx, &ys),
            ));
        }
    }
    s.push_str("  </g>\n");
}

/// Where to draw one edge label: the anchor the layout's placement engine
/// picked (M13), falling back to the midpoint of the polyline's longest
/// segment only for a hand-built [`EdgePath`] without one.
fn label_anchor(path: &EdgePath) -> (f32, f32) {
    path.label_at
        .unwrap_or_else(|| longest_segment_midpoint(&shortened_points(path)))
}

/// The midpoint of the longest segment of a polyline (by Euclidean length),
/// with ties going to the first (lowest-index) such segment. Only the
/// fallback anchor for a [`EdgePath`] without `label_at` (see
/// [`label_anchor`]); the placement policy itself lives in `layout.rs` (M13),
/// where the whole geometry is known.
fn longest_segment_midpoint(points: &[(f32, f32)]) -> (f32, f32) {
    if points.is_empty() {
        return (0.0, 0.0);
    }
    if points.len() == 1 {
        return points[0];
    }
    let mut best_i = 0usize;
    let mut best_len = -1.0_f32;
    for (i, w) in points.windows(2).enumerate() {
        let (ax, ay) = w[0];
        let (bx, by) = w[1];
        let l = (bx - ax).hypot(by - ay);
        if l > best_len {
            best_len = l;
            best_i = i;
        }
    }
    let (ax, ay) = points[best_i];
    let (bx, by) = points[best_i + 1];
    ((ax + bx) * 0.5, (ay + by) * 0.5)
}

/// Inner content of a `<text>` element for a possibly multi-line `label`
/// (lines separated by `\n`). Every line sits at `x`; `ys` gives each line's
/// baseline y (line 0's is the `<text>` element's own y, so it is skipped).
/// The first line is emitted inline and each further line becomes an
/// absolutely positioned `<tspan x y>` (explicit x/y make each tspan its own
/// text chunk, so inherited `text-anchor`/`dominant-baseline` apply per
/// line). A single-line label renders as just the escaped text — byte-
/// identical to the pre-multi-line output, so golden snapshots hold.
fn text_body(label: &str, x: f32, ys: &[f32]) -> String {
    let lines: Vec<&str> = label.split('\n').collect();
    let mut out = escape_xml(lines[0]);
    for (i, line) in lines.iter().enumerate().skip(1) {
        out.push_str(&format!(
            "<tspan x=\"{}\" y=\"{}\">{}</tspan>",
            fmt(x),
            fmt(ys[i]),
            escape_xml(line),
        ));
    }
    out
}

/// Baseline y for line `i` of an `n`-line text block centered on `cy` at
/// `font_size`: lines step by one [`text::line_height`] and the block
/// (first line's em box plus one line height per extra line) is centered.
fn centered_line_ys(cy: f32, n: usize, font_size: f32) -> Vec<f32> {
    let lh = text::line_height(font_size);
    let mid = (n - 1) as f32 / 2.0;
    (0..n).map(|i| cy + (i as f32 - mid) * lh).collect()
}

/// One node: a rounded rect (or cylinder) plus a centered label. The node's
/// `color` (stroke) and `fill` (interior) attributes, and the `cylinder`
/// shape, are honored (M6).
fn render_node(s: &mut String, node: &Node, rect: &NodeRect) {
    let stroke = escape_xml(node.color.as_deref().unwrap_or(NODE_STROKE));
    let fill = escape_xml(node.fill.as_deref().unwrap_or(FILL));
    let ink = escape_xml(node.text.as_deref().unwrap_or(INK));
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
    let ys = centered_line_ys(label_y, text::line_count(&node.label), crate::layout::FONT_SIZE);
    s.push_str(&format!(
        "    <text x=\"{}\" y=\"{}\" font-family=\"{FONT_FAMILY}\" font-size=\"{}\" fill=\"{ink}\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>\n",
        fmt(cx),
        fmt(ys[0]),
        crate::layout::FONT_SIZE,
        text_body(&node.label, cx, &ys),
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

    /// Pull every `y="…"` attribute value out of the first `<text …>`
    /// element (including its child tspans) whose content contains `needle`,
    /// in document order (line 0's y first, then tspan ys).
    fn text_ys(svg: &str, needle: &str) -> Vec<f32> {
        let chunk = svg
            .split("<text")
            .find(|chunk| chunk.contains(needle))
            .unwrap_or_else(|| panic!("no <text> containing {needle:?}"));
        let element = &chunk[..chunk.find("</text>").expect("<text> closed")];
        element
            .split(" y=\"")
            .skip(1)
            .map(|rest| {
                let y = &rest[..rest.find('"').expect("closed y attr")];
                y.parse::<f32>().expect("numeric y")
            })
            .collect()
    }

    #[test]
    fn multiline_node_label_renders_tspans() {
        let svg = render("diagram top-down\napp \"Kubernetes\\nCluster\"\n");
        // First line inline, second line as an absolutely positioned tspan.
        assert!(svg.contains("Kubernetes"), "first line inline");
        assert!(svg.contains("<tspan x="), "extra line becomes a tspan");
        assert!(svg.contains(">Cluster</tspan>"), "second line content");
        // The two lines are one node line-height apart.
        let ys = text_ys(&svg, "Kubernetes");
        assert_eq!(ys.len(), 2);
        assert!(
            (ys[1] - ys[0] - text::line_height(crate::layout::FONT_SIZE)).abs() < 1e-2,
            "lines should step by one line height: {ys:?}"
        );
    }

    #[test]
    fn single_line_labels_stay_plain() {
        // The tspan path is only for labels that actually contain newlines;
        // single-line output remains byte-identical to the old renderer.
        let svg = render("diagram top-down\na\n");
        assert!(!svg.contains("<tspan"));
        assert!(svg.contains(">a</text>"));
    }

    #[test]
    fn multiline_edge_label_renders_tspans() {
        let svg = render(
            "diagram top-down\n\n\
             a -- \"deploy\\nrollback\" --> b\n",
        );
        assert!(svg.contains("deploy"));
        assert!(svg.contains("<tspan x="));
        assert!(svg.contains(">rollback</tspan>"));
        let ys = text_ys(&svg, "deploy");
        assert_eq!(ys.len(), 2);
        assert!(
            (ys[1] - ys[0] - text::line_height(EDGE_LABEL_SIZE)).abs() < 1e-2,
            "edge label lines should step by one line height: {ys:?}"
        );
    }

    #[test]
    fn multiline_subgraph_title_renders_stacked_tspans() {
        let svg = render(
            "diagram top-down\n\
             subgraph \"Kubernetes\\nCluster\"\n\
             a\n\
             end\n",
        );
        // Stacked downward from the frame top: line 0 at FRAME_TITLE_Y, the
        // second one line height below (matching the inset frame_insets
        // reserves).
        let ys = text_ys(&svg, "Kubernetes");
        assert_eq!(ys.len(), 2);
        assert!(
            (ys[1] - ys[0] - text::line_height(FRAME_TITLE_SIZE)).abs() < 1e-2,
            "title lines should stack one line height apart: {ys:?}"
        );
        // The first line rides at the frame's top inset (FRAME_TITLE_Y).
        assert!(ys[0] > 0.0);
    }

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
        let e1 = svg.lines().find(|l| l.contains("<polyline")).unwrap();
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
    fn edge_line_ends_short_of_arrowhead_tip() {
        // The line is pulled back by `ARROW_BACKOFF` along its last segment so
        // it tucks under the arrowhead (a thick stroke's round cap would
        // otherwise poke past the tip). For a top-down a->b edge the last
        // segment is vertical, so the rendered polyline's final point sits
        // `ARROW_BACKOFF` above b's top edge.
        let raw = parser::parse_diagram("diagram top-down\na-->b\n").unwrap();
        let d = resolve::resolve(&raw).unwrap();
        let l = layout::layout(&d);
        let svg = render_svg(&d, &l);
        let b = l.nodes.iter().find(|n| n.id == "b").unwrap();
        let line = svg.lines().find(|ln| ln.contains("<polyline")).unwrap();
        let pts_attr = line.split("points=\"").nth(1).unwrap().split('"').next().unwrap();
        let last = pts_attr.split(' ').next_back().unwrap();
        let mut it = last.split(',');
        let lx: f32 = it.next().unwrap().parse().unwrap();
        let ly: f32 = it.next().unwrap().parse().unwrap();
        let cx = b.x + b.w / 2.0;
        assert!((lx - cx).abs() < 1e-2, "last x {lx} != b center {cx}");
        assert!(
            (ly - (b.y - ARROW_BACKOFF)).abs() < 1e-2,
            "line should end {ARROW_BACKOFF} above b's top ({:.2}), got y={ly}",
            b.y - ARROW_BACKOFF
        );
    }

    #[test]
    fn arrow_marker_refx_offsets_line_shortening() {
        // The marker's refX is reduced from the tip (10) by `ARROW_BACKOFF` so
        // the arrowhead tip still lands on the node boundary while the line
        // tucks under it.
        let svg = render("diagram top-down\na-->b\n");
        let want = fmt(10.0 - ARROW_BACKOFF);
        assert!(
            svg.contains(&format!("refX=\"{want}\"")),
            "marker refX should be {want} (10 - ARROW_BACKOFF):\n{svg}"
        );
        assert!(!svg.contains("refX=\"10\""), "refX should not be the bare tip 10");
    }

    #[test]
    fn shortened_points_pulls_back_and_clamps() {
        use crate::layout::EdgePath;
        // Normal segment: pulled back by ARROW_BACKOFF along the segment.
        let p = shortened_points(&EdgePath {
            from: "a".into(),
            to: "b".into(),
            points: vec![(0.0, 0.0), (0.0, 100.0)],
            label_at: None,
        });
        assert_eq!(p[1], (0.0, 100.0 - ARROW_BACKOFF));
        // Segment shorter than ARROW_BACKOFF: clamped, never flips backwards.
        let p = shortened_points(&EdgePath {
            from: "a".into(),
            to: "b".into(),
            points: vec![(0.0, 0.0), (0.0, 1.0)],
            label_at: None,
        });
        assert!(p[1].1 > 0.0 && p[1].1 < 1.0, "clamped end out of bounds: {:?}", p[1]);
        // Single point: unchanged.
        let p = shortened_points(&EdgePath {
            from: "a".into(),
            to: "b".into(),
            points: vec![(5.0, 5.0)],
            label_at: None,
        });
        assert_eq!(p, vec![(5.0, 5.0)]);
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
    fn longest_segment_midpoint_placement() {
        // Empty / single-point degenerate cases (the fallback anchor for a
        // hand-built path without `label_at`).
        assert_eq!(longest_segment_midpoint(&[]), (0.0, 0.0));
        assert_eq!(longest_segment_midpoint(&[(1.0, 2.0)]), (1.0, 2.0));
        // A single segment: its midpoint.
        assert_eq!(
            longest_segment_midpoint(&[(0.0, 0.0), (10.0, 0.0)]),
            (5.0, 0.0)
        );
        // A Z-route: the longest (horizontal) segment's midpoint, NOT the
        // arc-length midpoint (which would sit just past the first bend).
        //   seg0 vertical 10, seg1 horizontal 40, seg2 vertical 10.
        let z = vec![(0.0, 0.0), (0.0, 10.0), (40.0, 10.0), (40.0, 20.0)];
        assert_eq!(longest_segment_midpoint(&z), (20.0, 10.0));
        // Ties go to the first (lowest-index) longest segment.
        //   two segments of equal length 10.
        let tied = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(longest_segment_midpoint(&tied), (5.0, 0.0));
    }

    #[test]
    fn edge_labels_render_at_the_layout_anchor() {
        // M13 contract: the anchor is computed by the layout's placement
        // engine (policy tested in `layout.rs`) and stored on the `EdgePath`;
        // the renderer draws the text *and* its knockout rect at exactly that
        // point. Checks the infra "events" and "replication" labels.
        let raw = parser::parse_diagram(include_str!("../../examples/infra.mmd")).unwrap();
        let d = resolve::resolve(&raw).unwrap();
        let l = layout::layout(&d);
        let svg = render_svg(&d, &l);
        for label in ["events", "replication"] {
            let idx = d
                .edges
                .iter()
                .position(|e| e.label.as_deref() == Some(label))
                .unwrap();
            let (ax, ay) = l.edges[idx].label_at.expect("labeled edge has an anchor");
            // Text at the anchor (single-line label: y == anchor y).
            let text = svg
                .lines()
                .find(|t| t.contains(&format!(">{label}</text>")))
                .unwrap();
            assert!(text.contains(&format!("x=\"{}\"", fmt(ax))), "{label} text x: {text}");
            assert!(text.contains(&format!("y=\"{}\"", fmt(ay))), "{label} text y: {text}");
            // The knockout rect is centered on the anchor.
            let m = crate::text::measure(label, EDGE_LABEL_SIZE);
            let rw = m.width + 2.0 * LABEL_PAD;
            let rh = m.height + 2.0 * LABEL_PAD;
            let rect = svg
                .lines()
                .find(|t| t.starts_with("    <rect") && t.contains(&format!("x=\"{}\"", fmt(ax - rw / 2.0))))
                .unwrap();
            assert!(rect.contains(&format!("y=\"{}\"", fmt(ay - rh / 2.0))), "{label} rect y: {rect}");
            assert!(rect.contains(&format!("width=\"{}\"", fmt(rw))), "{label} rect w: {rect}");
            assert!(rect.contains(&format!("height=\"{}\"", fmt(rh))), "{label} rect h: {rect}");
        }
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
    fn snapshot_finance() {
        // A larger real-world diagram exercising M9: a `User --> VPN` edge that
        // must route around a peer `prd` node, plus a thick `User --> prd` edge
        // and several `VPN --> cvm` edges sharing VPN's bottom side (port
        // separation + lane separation) and a deep-target around-frame route.
        assert_snapshot("finance", &render(include_str!("../../examples/finance.mmd")));
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
    fn snapshot_sides() {
        // The M11 surface: forced page-space sides (`from=` / `to=`),
        // agreeable and contradictory alike, a forced self-loop, and a
        // forced side on a cross-boundary edge inside a left-right subgraph.
        assert_snapshot("sides", &render(include_str!("../../examples/sides.mmd")));
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

    // ---- M7: orthogonal routing in the SVG output ----

    /// Parse the `points="..."` of a `<polyline>` into a list of `(x, y)`.
    fn parse_polyline_points(line: &str) -> Vec<(f32, f32)> {
        let pts = line
            .split("points=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        pts.split(' ')
            .map(|p| {
                let mut it = p.split(',');
                (
                    it.next().unwrap().parse::<f32>().unwrap(),
                    it.next().unwrap().parse::<f32>().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn svg_edge_polylines_are_orthogonal() {
        // Every edge <polyline> in a rendered diagram must be axis-aligned
        // (M7), including the cross-boundary edges into the K8s subgraph.
        let svg = render(include_str!("../../examples/infra.mmd"));
        let polylines: Vec<&str> = svg.lines().filter(|l| l.contains("<polyline")).collect();
        assert!(!polylines.is_empty(), "infra should have edge polylines");
        for line in &polylines {
            let pts = parse_polyline_points(line);
            assert!(pts.len() >= 2, "polyline has < 2 points: {line}");
            for w in pts.windows(2) {
                let (ax, ay) = w[0];
                let (bx, by) = w[1];
                assert!(
                    (ax - bx).abs() < 1e-2 || (ay - by).abs() < 1e-2,
                    "non-orthogonal segment in polyline: {line}"
                );
            }
        }
    }

    #[test]
    fn arrow_marker_is_auto_oriented() {
        // The arrowhead marker keeps `orient="auto"` so the axis-aligned
        // final segment orients the arrow along a clean cardinal direction.
        let svg = render("diagram top-down\na-->b\n");
        assert!(svg.contains("orient=\"auto\""));
    }

    // ---- M7.5: subgraph color/fill/line/text and text-color rendering ----

    /// The frame `<rect>` for the (single) subgraph in a diagram. Frames use
    /// `rx="8"` (FRAME_RADIUS); node boxes use `rx="6"` (RADIUS), so this
    /// picks the frame out from any node boxes.
    fn frame_rect(svg: &str) -> &str {
        svg.lines()
            .find(|l| l.contains("<rect") && l.contains("rx=\"8\""))
            .unwrap_or_else(|| panic!("no subgraph frame <rect rx=\"8\"> in:\n{svg}"))
    }

    #[test]
    fn subgraph_color_fill_line_text_are_rendered() {
        let svg = render(
            "diagram top-down\n\
             subgraph \"S\" color=\"#888\" fill=\"#eef\" line=\"dashed\" text=\"#005\"\n\
             a\n\
             end\n",
        );
        let frame = frame_rect(&svg);
        assert!(frame.contains("fill=\"#eef\""), "subgraph fill not rendered: {frame}");
        assert!(frame.contains("stroke=\"#888\""), "subgraph border color not rendered: {frame}");
        assert!(frame.contains("stroke-dasharray=\"6 4\""), "dashed subgraph line not rendered: {frame}");
        let title = svg.lines().find(|l| l.contains(">S</text>")).unwrap();
        assert!(title.contains("fill=\"#005\""), "subgraph title text color not rendered: {title}");
    }

    #[test]
    fn subgraph_line_styles_render() {
        // thick doubles the frame stroke width (1 -> 2) and stays solid.
        let svg = render("diagram top-down\nsubgraph \"S\" line=\"thick\"\na\nend\n");
        let frame = frame_rect(&svg);
        assert!(frame.contains("stroke-width=\"2\""), "thick frame not wider: {frame}");
        assert!(!frame.contains("stroke-dasharray"), "thick frame should not be dashed: {frame}");
        // dotted reuses the same 1-4 dash as dotted edges.
        let svg = render("diagram top-down\nsubgraph \"S\" line=\"dotted\"\na\nend\n");
        let frame = frame_rect(&svg);
        assert!(frame.contains("stroke-dasharray=\"1 4\""), "dotted frame: {frame}");
        // solid is explicit but equivalent to the default.
        let svg = render("diagram top-down\nsubgraph \"S\" line=\"solid\"\na\nend\n");
        let frame = frame_rect(&svg);
        assert!(!frame.contains("stroke-dasharray"), "solid frame should have no dash: {frame}");
        assert!(frame.contains("stroke-width=\"1\""), "solid frame width: {frame}");
    }

    #[test]
    fn default_subgraph_frame_is_unstyled() {
        // A subgraph with no style attributes falls back to the default frame
        // palette: FRAME_FILL interior, FRAME_STROKE border, default width,
        // no dash, and a title that inherits the group's default fill.
        let svg = render("diagram top-down\nsubgraph \"S\"\na\nend\n");
        let frame = frame_rect(&svg);
        assert!(frame.contains("fill=\"#f2f8f4\""), "{frame}");
        assert!(frame.contains("stroke=\"#88BDA4\""), "{frame}");
        assert!(frame.contains("stroke-width=\"1\""), "{frame}");
        assert!(!frame.contains("stroke-dasharray"), "{frame}");
        let title = svg.lines().find(|l| l.contains(">S</text>")).unwrap();
        assert!(!title.contains("fill="), "default title should inherit the group fill: {title}");
    }

    #[test]
    fn default_node_colors() {
        // A node with no `color`/`fill` attributes falls back to the default
        // node palette: NODE_STROKE border and FILL interior. (Edges keep the
        // separate default stroke; only node boxes use NODE_STROKE.)
        let svg = render("diagram top-down\na \"A\"\n");
        let box_line = svg.lines().find(|l| l.contains("rx=\"6\"")).unwrap();
        assert!(box_line.contains("fill=\"#c0e1fc\""), "default node fill: {box_line}");
        assert!(box_line.contains("stroke=\"#2196F3\""), "default node stroke: {box_line}");
    }

    #[test]
    fn node_text_color_is_rendered() {
        let svg = render("diagram top-down\na \"A\" text=\"#0055ff\"\n");
        let text = svg.lines().find(|l| l.contains(">A</text>")).unwrap();
        assert!(text.contains("fill=\"#0055ff\""), "node text color not rendered: {text}");
    }

    #[test]
    fn default_node_text_color_is_ink() {
        let svg = render("diagram top-down\na \"A\"\n");
        let text = svg.lines().find(|l| l.contains(">A</text>")).unwrap();
        assert!(text.contains("fill=\"#222\""), "default node text should be INK #222: {text}");
    }

    #[test]
    fn edge_text_color_is_rendered() {
        let svg = render("diagram top-down\na -- \"sync\" text=\"#005\" --> b\n");
        let text = svg.lines().find(|l| l.contains(">sync</text>")).unwrap();
        assert!(text.contains("fill=\"#005\""), "edge label text color not rendered: {text}");
    }

    #[test]
    fn default_edge_label_text_color_is_ink() {
        // A default edge label inherits the label group's `fill` (INK) rather
        // than carrying its own, so the output stays minimal.
        let svg = render("diagram top-down\na -- \"sync\" --> b\n");
        let text = svg.lines().find(|l| l.contains(">sync</text>")).unwrap();
        assert!(!text.contains("fill="), "default edge label should inherit the group fill: {text}");
    }

    #[test]
    fn subgraph_fill_paints_behind_contents() {
        // The frame is drawn before edges and nodes, so a filled subgraph is
        // a background behind its members (the member node's own fill sits
        // on top). Confirm the frame rect precedes the node rect in the SVG.
        let svg = render("diagram top-down\nsubgraph \"S\" fill=\"#eef\"\na\nend\n");
        assert!(svg.find("rx=\"8\"").unwrap() < svg.find("rx=\"6\"").unwrap());
        let frame = svg.lines().find(|l| l.contains("rx=\"8\"")).unwrap();
        let node = svg.lines().find(|l| l.contains("rx=\"6\"")).unwrap();
        assert!(frame.contains("fill=\"#eef\""), "frame fill: {frame}");
        assert!(node.contains("fill=\"#c0e1fc\""), "node keeps its own default fill: {node}");
    }

    #[test]
    fn snapshot_subgraph_style() {
        // The full M7.5 surface in one diagram: subgraph color/fill/line/text,
        // plus node and edge text colors. Frames paint behind contents.
        assert_snapshot(
            "subgraph_style",
            &render(
                "diagram top-down\n\
                 a \"A\" text=\"#0055ff\"\n\
                 b \"B\"\n\
                 subgraph \"Group\" color=\"#0a7\" fill=\"#cfe\" line=\"dashed\" text=\"#005\"\n\
                 c \"C\"\n\
                 d \"D\" text=\"#a00\"\n\
                 end\n\
                 a -- \"sync\" text=\"#0a7\" --> c\n\
                 b --> d\n",
            ),
        );
    }
}
