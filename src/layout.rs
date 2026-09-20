//! Layered (Sugiyama-style) layout, extended for **compound graphs** with
//! per-subgraph direction (milestones 2 and 4).
//!
//! The flat engine is the classic four-phase framework, unchanged from M2:
//!
//! 1. **Cycle removal** — a DFS that reverses back-edges so the graph becomes
//!    a DAG (the reversal is internal; the original edge direction is
//!    preserved in the output).
//! 2. **Layering** — longest-path ranking.
//! 3. **Crossing minimization** — barycenter sweeps, keeping the best ordering.
//!    Long edges are routed through zero-size *dummy* nodes.
//! 4. **Coordinate assignment** — centered block x-placement (each
//!    non-source rank centered on its parents' centroid; overlap-free) and
//!    per-rank band y-placement.
//!
//! Everything is computed in a *canonical* top-down space (rank grows
//! downward, order is horizontal), then a rigid transform maps node
//! rectangles and edge waypoints into the requested direction. The same
//! transform is applied to node rects and waypoint points, so ports stay
//! glued to the correct side of each box in every direction.
//!
//! ## Compound layout (M4)
//!
//! Subgraphs form a containment tree (each node belongs to at most one
//! subgraph; subgraphs nest). Layout is **recursive**: each level (the
//! top-level diagram, or a subgraph) is laid out as a flat graph whose
//! "items" are the level's direct-child real nodes plus its direct-child
//! subgraphs (treated as opaque compound boxes). A subgraph box is sized to
//! fit its own recursively-laid-out contents plus a labeled frame. Each
//! level uses its own effective direction (a subgraph's direction, or the
//! direction inherited from its enclosing level).
//!
//! Each original edge is assigned to the **lowest common ancestor (LCA)**
//! level of its two endpoints: the unique level that contains both endpoints
//! (recursively). At that level the edge is represented between the two
//! endpoints' *representatives* — the direct child of the LCA that is an
//! ancestor-or-self of each endpoint. So an edge between a top-level node and
//! a node deep inside a subgraph becomes, at the top level, an edge between
//! that node and the subgraph's compound box. This keeps compound boxes
//! positioned relative to their connected neighbors without dragging the
//! inner nodes' internal arrangement around — the Mermaid/dagre failure mode
//! this tool exists to escape.
//!
//! ### Rendering rules (v0/M5)
//!
//! - An edge whose two representatives are both **real nodes** (a direct
//!   internal edge of its LCA level) is rendered with the flat engine's
//!   waypoints, in absolute coordinates.
//! - Any other edge (at least one representative is a compound box) is a
//!   **cross-boundary** edge. It is routed through **connection points on the
//!   subgraph frames** it crosses, without disturbing the groups' internal
//!   layout (the M5 headline guarantee — the failure mode this tool exists to
//!   escape). The LCA-level segment connects the two representatives' *ports*:
//!   each rep port sits at the **endpoint node's** cross-coordinate on its
//!   frame's facing side (not the frame's center), so a within-frame stub runs
//!   straight along the node's own axis and edges entering a subgraph line up
//!   with their target node rather than all converging on the frame's midpoint.
//!   When that point falls under the frame's (top-left) title the stub detours
//!   just past the title and jogs below it; a stub that would pierce a sibling
//!   (the target sits beyond other members along the frame's flow axis) routes
//!   around the frame. Stub segments that traverse intermediate (nested)
//!   frames are clipped at each frame's boundary so the path records where it
//!   crosses.
//!
//! ## Orthogonal edge routing (M7)
//!
//! Every edge — direct or cross-boundary — is finally passed through
//! [`ortho_chain`], which turns each diagonal segment into a right-angle "Z":
//! along the edge's flow axis (the rank axis in page space — vertical for
//! top-down / bottom-up, horizontal for left-right / right-left) to the
//! segment's flow midpoint, across to the target's cross coordinate, then
//! along the flow axis to the target. The perpendicular jog lands between the
//! two waypoints' flow coordinates — in an inter-rank gap or a frame's
//! padding — so it stays clear of node interiors and the M5 "stub does not
//! cross a sibling" guarantee holds. Direct edges use the midpoint bias via
//! [`orthogonalize`]; a cross-boundary within-frame stub that would cross a
//! titled frame's TOP side instead enters past the title and jogs a fixed
//! [`STUB_JOG_CLEARANCE`] above the node (in the clear padding below the
//! title) — see [`title_detour`]. The arrowhead keeps its `orient="auto"`
//! marker, which the now axis-aligned final segment orients along a clean
//! cardinal direction.

use crate::ast::{Diagram, Direction, Subgraph};
use crate::text;

// ---- Tunable constants (pixels) ----

/// Label font size. Public so the SVG renderer sizes its `<text>` to match
/// the boxes laid out from these metrics.
pub const FONT_SIZE: f32 = 14.0;
/// Horizontal padding inside a node box, each side.
const NODE_PAD_X: f32 = 12.0;
/// Vertical padding inside a node box, each side.
const NODE_PAD_Y: f32 = 7.0;
/// Minimum gap between two items in the same layer (edge to edge).
const NODE_SEP: f32 = 30.0;
/// Minimum gap between two consecutive layers (edge to edge).
const RANK_GAP: f32 = 45.0;
/// Page margin around the whole drawing (top level only).
const MARGIN: f32 = 20.0;
/// Smallest node size, so even tiny labels get a visible box.
const MIN_NODE_W: f32 = 40.0;
const MIN_NODE_H: f32 = 28.0;
/// Elliptical cap radius (the lid and base rim) for `cylinder` nodes, in
/// pixels. Fixed (not scaled to the box) so that growing a cylinder's height
/// actually buys the label more room instead of also growing the caps. Public
/// so the SVG renderer draws the caps at the same radius the layout sized for.
pub const CYL_RY: f32 = 5.0;
/// Extra vertical breathing room inside a cylinder's straight-sided body,
/// each side, beyond a box's [`NODE_PAD_Y`]. The caps dip `2·CYL_RY` into the
/// box from top and bottom; this padding keeps the centered label clear of the
/// lid's bottom curve (see [`cyl_height`]).
const CYL_PAD: f32 = 2.0;
/// Iteration count for crossing minimization. Deterministic; chosen
/// generously for small graphs.
const CROSS_ITERS: usize = 24;

// Frame (subgraph) sizing, in pixels. The frame is a labeled border drawn
// around a subgraph's contents; these define the inset between the frame
// edge and the inner content (the recursive layout's bounding box).
const FRAME_PAD_X: f32 = 10.0;
const FRAME_PAD_Y: f32 = 8.0;
/// Height reserved at the top of a frame for the title, when present.
const FRAME_TITLE_H: f32 = 20.0;
/// Inset of the title text from the frame's top-left corner, and the font
/// size it is rendered at. Layout reads these only to detect when a
/// cross-boundary within-frame stub would cross the title text (see
/// [`title_detour_clear_x`]) so it can route around it. Keep in sync with
/// the renderer's `FRAME_TITLE_X` / `FRAME_TITLE_SIZE` (both in
/// `render/svg.rs`): the title text occupies roughly `x ∈ [frame.x +
/// FRAME_TITLE_X, frame.x + FRAME_TITLE_X + title_width]` at the top of
/// the frame.
const FRAME_TITLE_X: f32 = 10.0;
const FRAME_TITLE_FONT_SIZE: f32 = 12.0;
/// Extra padding added to a subgraph frame's top inset *and* bottom padding
/// when a cross-boundary edge connects to one of the frame's *immediate*
/// children (a direct-child node the edge reaches by crossing the frame).
/// It grows the frame along the LCA-level flow axis so the edge's within-
/// frame stub has room to jog: clear of the title text above the node and
/// clear of the node's arrowhead below the jog (see [`STUB_JOG_CLEARANCE`]).
/// One subgraph-title font height each side — the title font is smaller than
/// the node label font, so this is a modest growth; a subgraph with no such
/// edges keeps the default geometry. Keep this in sync with the renderer's
/// subgraph title font size (`FRAME_TITLE_SIZE` in `render/svg.rs`, 12 px):
/// it is sized to one title-height of room.
const CROSS_FRAME_PAD: f32 = 12.0;
/// Distance from a node's port at which a title-detour within-frame stub
/// places its right-angle jog, along the flow axis toward the frame edge.
/// Used only for stubs that cross a titled frame's TOP side (see
/// [`title_detour`]): the jog lands in the clear padding just below the
/// title instead of at the segment midpoint (which would sit on the title
/// text).
///
/// This must exceed two things in `render/svg.rs`. First, the line-end
/// shortening (`ARROW_BACKOFF`, 4 px) so the stub's final segment stays
/// longer than the shortening and the arrowhead tip lands on the node
/// boundary rather than poking into the node. Second, the arrowhead's own
/// extent back from the tip (`markerHeight`, 10 px — the marker's base sits
/// `markerHeight` short of the tip), so the jog clears the arrowhead body and
/// the horizontal approach line does not cut into the arrowhead's side. With
/// [`CROSS_FRAME_PAD`] growing the frame, this also leaves the jog clear of
/// the title text above. It is a layout/render shared constant in the same
/// spirit as [`FONT_SIZE`] and [`CYL_RY`]; keep it in sync if the renderer's
/// marker or backoff changes.
const STUB_JOG_CLEARANCE: f32 = 12.0;
/// How far a title-detour entry sits past the right edge of a subgraph's
/// title text (see [`title_detour_clear_x`]). The within-frame entry runs
/// along the frame's top band at this `x`, so it must clear the title's last
/// glyph; this gap comfortably clears the 1.5 px stroke and a little
/// breathing room.
const TITLE_CLEAR_GAP: f32 = 6.0;

// ---- Public output types (consumed by milestone 3 rendering) ----

/// A laid-out diagram: node rectangles, edge polylines, subgraph frames, and
/// the canvas size. Nodes and edges correspond by index to the resolved
/// [`Diagram`] (declaration order); subgraphs correspond by index to
/// `diagram.subgraphs`.
#[derive(Debug, Clone)]
pub struct Layout {
    pub nodes: Vec<NodeRect>,
    pub edges: Vec<EdgePath>,
    pub subgraphs: Vec<SubgraphRect>,
    pub width: f32,
    pub height: f32,
}

/// A node's axis-aligned rectangle. `x`/`y` is the top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRect {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// An edge as an ordered polyline of waypoints from `from` to `to`. The first
/// point sits on the source node's port, the last on the target node's port;
/// intermediate points (if any) route the edge through the gaps between layers.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePath {
    pub from: String,
    pub to: String,
    pub points: Vec<(f32, f32)>,
}

/// A subgraph's frame rectangle, in absolute (page) coordinates. The
/// frame's style (title, border color/fill/line, title text color) lives on
/// the resolved [`crate::ast::Subgraph`]; the renderer zips `diagram.subgraphs`
/// with `layout.subgraphs` (they correspond by index) so this struct only
/// carries geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct SubgraphRect {
    /// Index into `Diagram::subgraphs`.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

// ---- Internal graph (index-based, shared by the flat engine) ----

/// A real or dummy node in the layered graph. (`id` is not needed by the
/// algorithm — output is keyed by item index.)
struct LNode {
    w: f32,
    h: f32,
    rank: usize,
}

/// One edge tracked through the flat pipeline.
struct OrigEdge {
    /// Original source/target item indices (for output + arrow direction).
    from: usize,
    to: usize,
    /// Whether cycle removal reversed this edge for the DAG.
    reversed: bool,
    /// The DAG orientation: `dag_low` is the lower-rank end, `dag_high` the
    /// higher-rank end (`rank[dag_low] < rank[dag_high]`).
    dag_low: usize,
    dag_high: usize,
    /// Path of item indices along the (possibly dummy-filled) DAG edge, in
    /// rank order low → high. Filled in during dummy insertion.
    path: Vec<usize>,
}

/// A flat graph item: a real node or an opaque compound box, with a fixed
/// size. The flat engine does not care which.
struct FlatItem {
    w: f32,
    h: f32,
}

/// A flat edge between two item indices, in the original (arrow) direction.
struct FlatEdge {
    from: usize,
    to: usize,
}

/// Result of laying out one flat graph.
struct FlatResult {
    /// Per item: `(x, y, w, h)` in the requested direction's final orientation.
    rects: Vec<(f32, f32, f32, f32)>,
    /// Per input edge: waypoints in the final orientation, running `from` → `to`.
    edges: Vec<FlatEdgeOut>,
    width: f32,
    height: f32,
}

struct FlatEdgeOut {
    #[allow(dead_code)]
    from: usize,
    #[allow(dead_code)]
    to: usize,
    points: Vec<(f32, f32)>,
}

// ---- Compound-layout support ----

/// Identifies an item at a level: a direct-child real node, or a direct-child
/// subgraph (compound box). Carries the *global* index into `Diagram::nodes`
/// or `Diagram::subgraphs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ItemRef {
    Node(usize),
    Sub(usize),
}

/// Per-edge precomputed routing info.
struct EdgeInfo {
    /// The LCA level that owns this edge (`None` = top level).
    lca: Option<usize>,
    /// Representative of each endpoint at the LCA level.
    rep_from: ItemRef,
    rep_to: ItemRef,
    /// Both representatives are real nodes (a direct internal edge).
    is_direct: bool,
}

/// The laid-out contents of a single level, in that level's local coordinate
/// system. For the top level, "local" is page coordinates. For a subgraph,
/// "local" is the subgraph's inner content space (origin at the frame's
/// content top-left); the parent translates it into place.
struct LevelOut {
    /// `(global node index, x, y, w, h)` for every descendant real node.
    nodes: Vec<(usize, f32, f32, f32, f32)>,
    /// `(global subgraph index, x, y, w, h)` for every descendant frame.
    frames: Vec<(usize, f32, f32, f32, f32)>,
    /// `(global edge index, waypoints)` for direct internal edges (rendered
    /// via flat waypoints). Cross-boundary edges are filled in later.
    edges: Vec<(usize, Vec<(f32, f32)>)>,
    /// Content size of this level (the flat canvas, including its margin).
    w: f32,
    h: f32,
}

/// A recursively-laid-out child subgraph, pending placement into its parent.
struct ChildOut {
    out: LevelOut,
    inner_ox: f32,
    inner_oy: f32,
}

// ---- Public entry point ----

/// Box height for a node of `shape` with a label of `text_height` pixels:
/// the label plus vertical padding, floored at [`MIN_NODE_H`]. For cylinders
/// the straight-sided body is sized like a box (same text padding) plus an
/// extra [`CYL_PAD`] each side, and the two elliptical caps ([`CYL_RY`] each,
/// but dipping `2·CYL_RY` into the box) are added on top and bottom. With
/// `dominant-baseline="central"` the centered label's glyph extent is the em
/// box (`text_height` tall, centered), so the clearance from the lid's
/// lowest point is `NODE_PAD_Y + CYL_PAD - CYL_RY` — positive and comfortable,
/// unlike a plain box where the caps would eat the text room.
fn cyl_height(shape: crate::ast::Shape, text_height: f32) -> f32 {
    use crate::ast::Shape;
    match shape {
        Shape::Box => (text_height + 2.0 * NODE_PAD_Y).max(MIN_NODE_H),
        Shape::Cylinder => (text_height + 2.0 * NODE_PAD_Y + 2.0 * CYL_PAD + 2.0 * CYL_RY)
            .max(MIN_NODE_H + 2.0 * CYL_RY),
    }
}

/// Lay out a resolved [`Diagram`], honoring subgraph containment and
/// per-subgraph direction.
pub fn layout(diagram: &Diagram) -> Layout {
    if diagram.nodes.is_empty() {
        // Still produce frames for any (empty) subgraphs, sized to their
        // padding, so they render as small labeled boxes rather than vanish.
        let top = layout_level(diagram, None, diagram.direction, &[], &Default::default());
        return assemble(diagram, top, &[]);
    }

    // Map node id -> index, matching `diagram.nodes` order.
    let id_index: std::collections::HashMap<&str, usize> = diagram
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i))
        .collect();

    // Precompute each edge's LCA level and representatives.
    let edge_infos: Vec<EdgeInfo> = diagram
        .edges
        .iter()
        .map(|e| {
            let fi = id_index[e.from.as_str()];
            let ti = id_index[e.to.as_str()];
            let lca = lca_level(diagram.nodes[fi].group, diagram.nodes[ti].group, &diagram.subgraphs);
            let rep_from = rep_of(fi, lca, diagram);
            let rep_to = rep_of(ti, lca, diagram);
            let is_direct = matches!(rep_from, ItemRef::Node(_)) && matches!(rep_to, ItemRef::Node(_));
            EdgeInfo {
                lca,
                rep_from,
                rep_to,
                is_direct,
            }
        })
        .collect();

    // Subgraphs that have a cross-boundary edge to an *immediate* child (a
    // direct-child node the edge reaches by crossing the frame). These get
    // extra frame padding ([`CROSS_FRAME_PAD`]) so the edge's within-frame
    // stub has room to jog clear of the title and the node's arrowhead. A
    // subgraph reached only through deeper descendants does not qualify:
    // the stub's jog lives in the innermost frame's padding, not this one's.
    let cross_subs = cross_boundary_subs(diagram, &edge_infos, &id_index);

    let top = layout_level(diagram, None, diagram.direction, &edge_infos, &cross_subs);
    assemble(diagram, top, &edge_infos)
}

/// Turn a top-level [`LevelOut`] (page coordinates) into the public
/// [`Layout`], routing cross-boundary edges through frame connection points
/// (M5), then orthogonalizing every edge (M7) into right-angle segments.
/// Direct internal edges keep the flat engine's waypoints; every other edge
/// is rebuilt as a frame-aware path in [`cross_boundary_path`]. Both kinds
/// are finally passed through [`orthogonalize`] with the edge's own level
/// flow axis (from [`effective_direction`] of its LCA).
fn assemble(diagram: &Diagram, top: LevelOut, edge_infos: &[EdgeInfo]) -> Layout {
    use std::collections::HashMap;

    let id_index: HashMap<&str, usize> = diagram
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i))
        .collect();

    let mut node_rect: HashMap<usize, (f32, f32, f32, f32)> = HashMap::new();
    for (gi, x, y, w, h) in &top.nodes {
        node_rect.insert(*gi, (*x, *y, *w, *h));
    }

    // Absolute frame rect per subgraph index, for cross-boundary routing.
    let frame_rect: HashMap<usize, (f32, f32, f32, f32)> = top
        .frames
        .iter()
        .map(|(idx, x, y, w, h)| (*idx, (*x, *y, *w, *h)))
        .collect();

    // Immediate children (real-node rects + nested subgraph frame rects, in
    // absolute coordinates) per subgraph index — used by cross-boundary
    // routing to detect when a within-frame stub would pierce a sibling and
    // to route around the frame when it would.
    let mut frame_children: HashMap<usize, Vec<(f32, f32, f32, f32)>> = HashMap::new();
    for (gi, nd) in diagram.nodes.iter().enumerate() {
        if let Some(g) = nd.group {
            frame_children
                .entry(g)
                .or_default()
                .push(node_rect.get(&gi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0)));
        }
    }
    for (si, sg) in diagram.subgraphs.iter().enumerate() {
        if let Some(p) = sg.parent {
            frame_children
                .entry(p)
                .or_default()
                .push(frame_rect.get(&si).copied().unwrap_or((0.0, 0.0, 0.0, 0.0)));
        }
    }

    let mut direct_pts: HashMap<usize, Vec<(f32, f32)>> = HashMap::new();
    for (ei, pts) in &top.edges {
        direct_pts.insert(*ei, pts.clone());
    }

    let mut nodes_out = Vec::with_capacity(diagram.nodes.len());
    for (gi, n) in diagram.nodes.iter().enumerate() {
        let (x, y, w, h) = node_rect
            .get(&gi)
            .copied()
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        nodes_out.push(NodeRect {
            id: n.id.clone(),
            x,
            y,
            w,
            h,
        });
    }

    let mut edges_out = Vec::with_capacity(diagram.edges.len());
    for (ei, e) in diagram.edges.iter().enumerate() {
        // M7: every edge is finally orthogonalized along its level's flow
        // axis (the rank axis in page space), so diagonal segments become
        // right-angle bends. The flow axis is read from the edge's LCA-level
        // effective direction — the same direction the (already transformed)
        // waypoints were laid out under — so an edge inside a left-right
        // subgraph jogs horizontally even in a top-down diagram.
        let lca = edge_infos.get(ei).map(|i| i.lca).unwrap_or(None);
        let flow = FlowAxis::from_direction(effective_direction(lca, diagram));
        let raw = if let Some(pts) = direct_pts.get(&ei) {
            pts.clone()
        } else {
            // Cross-boundary edge (M5): route through connection points on the
            // subgraph frames it crosses, without disturbing the groups'
            // internal layout. The LCA-level segment connects the two
            // representatives' ports at the endpoint nodes' cross-coordinates
            // on their frames' facing sides (not the frame centers); within-
            // frame stubs run straight from each endpoint node to its
            // representative's port, detouring past a title or around a
            // pierced sibling as needed, and clipping at any intermediate
            // (nested) frame boundaries. The resulting (possibly diagonal)
            // segments are orthogonalized below.
            let fi = id_index[e.from.as_str()];
            let ti = id_index[e.to.as_str()];
            let from = node_rect.get(&fi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            let to = node_rect.get(&ti).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            let lca_dir = effective_direction(lca, diagram);
            let from_chain =
                chain_to_lca(diagram.nodes[fi].group, lca, &diagram.subgraphs);
            let to_chain =
                chain_to_lca(diagram.nodes[ti].group, lca, &diagram.subgraphs);
            cross_boundary_path(from, to, &from_chain, &to_chain, lca_dir, &|s| {
                frame_rect.get(&s).copied().unwrap_or((0.0, 0.0, 0.0, 0.0))
            }, &|s| {
                diagram
                    .subgraphs
                    .get(s)
                    .and_then(|sg| sg.title.as_ref())
                    .map(|t| text::measure(t, FRAME_TITLE_FONT_SIZE).width)
            }, &|s| {
                frame_children.get(&s).cloned().unwrap_or_default()
            })
        };
        let points = orthogonalize(&raw, flow);
        edges_out.push(EdgePath {
            from: e.from.clone(),
            to: e.to.clone(),
            points,
        });
    }

    let mut subgraphs_out = Vec::with_capacity(diagram.subgraphs.len());
    for gi in 0..diagram.subgraphs.len() {
        let (x, y, w, h) = top
            .frames
            .iter()
            .find(|(idx, _, _, _, _)| *idx == gi)
            .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        subgraphs_out.push(SubgraphRect {
            index: gi,
            x,
            y,
            w,
            h,
        });
    }

    Layout {
        nodes: nodes_out,
        edges: edges_out,
        subgraphs: subgraphs_out,
        width: top.w,
        height: top.h,
    }
}

/// Lay out one level (the top level `None`, or a subgraph index) and all of
/// its descendants. Returns the level's contents in its local coordinate
/// system.
fn layout_level(
    diagram: &Diagram,
    level: Option<usize>,
    dir: Direction,
    edge_infos: &[EdgeInfo],
    cross_subs: &std::collections::HashSet<usize>,
) -> LevelOut {
    // Direct-child subgraphs of this level.
    let child_subs: Vec<usize> = diagram
        .subgraphs
        .iter()
        .enumerate()
        .filter(|(_, sg)| sg.parent == level)
        .map(|(i, _)| i)
        .collect();

    // Recurse into each child subgraph first (bottom-up sizing).
    let mut child_outs: std::collections::HashMap<usize, ChildOut> =
        std::collections::HashMap::new();
    for &cs in &child_subs {
        let child_dir = diagram.subgraphs[cs].direction.unwrap_or(dir);
        let out = layout_level(diagram, Some(cs), child_dir, edge_infos, cross_subs);
        let (top_inset, _bot_pad) =
            frame_insets(&diagram.subgraphs[cs], cross_subs.contains(&cs));
        let inner_ox = FRAME_PAD_X;
        let inner_oy = top_inset;
        child_outs.insert(cs, ChildOut { out, inner_ox, inner_oy });
    }

    // Build this level's flat items: direct-child real nodes, then child
    // subgraphs (as opaque compound boxes).
    let mut items: Vec<FlatItem> = Vec::new();
    let mut item_refs: Vec<ItemRef> = Vec::new();
    let mut item_of: std::collections::HashMap<ItemRef, usize> = std::collections::HashMap::new();
    for (gi, node) in diagram.nodes.iter().enumerate() {
        if node.group == level {
            let m = text::measure(&node.label, FONT_SIZE);
            let w = (m.width + 2.0 * NODE_PAD_X).max(MIN_NODE_W);
            let h = cyl_height(node.shape, m.height);
            item_of.insert(ItemRef::Node(gi), items.len());
            item_refs.push(ItemRef::Node(gi));
            items.push(FlatItem { w, h });
        }
    }
    for &cs in &child_subs {
        let child = child_outs.get(&cs).expect("child out just inserted");
        let (top_inset, bot_pad) =
            frame_insets(&diagram.subgraphs[cs], cross_subs.contains(&cs));
        let frame_w = child.out.w + 2.0 * FRAME_PAD_X;
        let frame_h = child.out.h + top_inset + bot_pad;
        item_of.insert(ItemRef::Sub(cs), items.len());
        item_refs.push(ItemRef::Sub(cs));
        items.push(FlatItem {
            w: frame_w,
            h: frame_h,
        });
    }

    // Build the flat edges owned by this level (LCA == level), mapped to item
    // indices via their representatives.
    let mut flat_edges: Vec<FlatEdge> = Vec::new();
    let mut edge_global: Vec<usize> = Vec::new();
    for ei in 0..diagram.edges.len() {
        // For the empty-diagram path `edge_infos` is empty; skip safely.
        if ei >= edge_infos.len() {
            break;
        }
        if edge_infos[ei].lca != level {
            continue;
        }
        let info = &edge_infos[ei];
        let from_item = item_of[&info.rep_from];
        let to_item = item_of[&info.rep_to];
        flat_edges.push(FlatEdge {
            from: from_item,
            to: to_item,
        });
        edge_global.push(ei);
    }

    let margin = if level.is_none() { MARGIN } else { 0.0 };
    let flat = layout_flat(&items, &flat_edges, dir, margin);

    let mut out = LevelOut {
        nodes: Vec::new(),
        frames: Vec::new(),
        edges: Vec::new(),
        w: flat.width,
        h: flat.height,
    };

    // Place items. Real nodes record their rect; subgraph boxes record the
    // frame rect and then translate their already-computed contents into
    // this level's local space.
    for (item_idx, &(rx, ry, rw, rh)) in flat.rects.iter().enumerate() {
        match item_refs[item_idx] {
            ItemRef::Node(gi) => out.nodes.push((gi, rx, ry, rw, rh)),
            ItemRef::Sub(cs) => {
                out.frames.push((cs, rx, ry, rw, rh));
                let child = child_outs.remove(&cs).expect("child out present for its item");
                let ox = rx + child.inner_ox;
                let oy = ry + child.inner_oy;
                for (gi, x, y, w, h) in child.out.nodes {
                    out.nodes.push((gi, x + ox, y + oy, w, h));
                }
                for (cs2, x, y, w, h) in child.out.frames {
                    out.frames.push((cs2, x + ox, y + oy, w, h));
                }
                for (ei2, pts) in child.out.edges {
                    let tpts: Vec<(f32, f32)> =
                        pts.into_iter().map(|(px, py)| (px + ox, py + oy)).collect();
                    out.edges.push((ei2, tpts));
                }
            }
        }
    }

    // Direct internal edges owned by this level: take the flat engine's
    // waypoints (in this level's local space). Cross-boundary edges (whose
    // reps include a compound) are skipped here and drawn later as straight
    // lines in `assemble`.
    for (j, fe) in flat.edges.iter().enumerate() {
        let gi = edge_global[j];
        if edge_infos[gi].is_direct {
            out.edges.push((gi, fe.points.clone()));
        }
    }

    out
}

// ============ Containment helpers ============

/// Ancestor chain of a node's container, innermost first, ending with the
/// root (`None` = top level). Used to find the LCA level of two nodes.
fn ancestors(group: Option<usize>, subgraphs: &[crate::ast::Subgraph]) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    let mut cur = group;
    while let Some(idx) = cur {
        out.push(cur);
        cur = subgraphs[idx].parent;
    }
    out.push(None);
    out
}

/// Lowest common ancestor level of two nodes. `None` means the top level.
fn lca_level(
    a: Option<usize>,
    b: Option<usize>,
    subgraphs: &[crate::ast::Subgraph],
) -> Option<usize> {
    let ca = ancestors(a, subgraphs);
    let cb = ancestors(b, subgraphs);
    for x in &ca {
        if cb.contains(x) {
            return *x;
        }
    }
    None
}

/// The direct child of `level` that is an ancestor-or-self of `node_idx`.
/// `level` must be an ancestor of the node (the caller guarantees this via
/// the LCA computation).
fn rep_of(node_idx: usize, level: Option<usize>, diagram: &Diagram) -> ItemRef {
    let g = diagram.nodes[node_idx].group;
    if g == level {
        return ItemRef::Node(node_idx);
    }
    let mut cur = g;
    while let Some(idx) = cur {
        if diagram.subgraphs[idx].parent == level {
            return ItemRef::Sub(idx);
        }
        cur = diagram.subgraphs[idx].parent;
    }
    // `level` was not an ancestor of the node — should be unreachable.
    ItemRef::Node(node_idx)
}

/// The subgraph frames containing `node_group`, innermost first, up to (but
/// excluding) `lca` — i.e. the frames a cross-boundary edge crosses on this
/// side. An empty result means `node_group == lca`, so the node is a direct
/// child of the LCA and is its own representative.
fn chain_to_lca(
    node_group: Option<usize>,
    lca: Option<usize>,
    subgraphs: &[Subgraph],
) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut cur = node_group;
    while cur != lca {
        match cur {
            Some(idx) => {
                chain.push(idx);
                cur = subgraphs[idx].parent;
            }
            None => break,
        }
    }
    chain
}

/// The effective layout direction of a level: its own direction, or the one
/// inherited from its enclosing level (recursively up to the diagram's global
/// direction). Matches the `dir` parameter [`layout_level`] drives each level
/// with, but reconstructed here for an arbitrary level so [`assemble`] can
/// route cross-boundary edges at their LCA level without the flat engine's
/// waypoints.
fn effective_direction(level: Option<usize>, diagram: &Diagram) -> Direction {
    match level {
        None => diagram.direction,
        Some(idx) => diagram.subgraphs[idx]
            .direction
            .unwrap_or_else(|| effective_direction(diagram.subgraphs[idx].parent, diagram)),
    }
}

/// The subgraphs that have at least one cross-boundary edge to an *immediate*
/// child — a direct-child node the edge reaches by crossing the subgraph's
/// frame. Such frames get extra padding ([`CROSS_FRAME_PAD`]) so the edge's
/// within-frame stub has room to jog clear of the title and the node's
/// arrowhead. A subgraph reached only through deeper descendants does not
/// qualify: the stub's jog lives in the innermost frame's padding, not this
/// one's.
///
/// `id_index` maps node ids to their index in `diagram.nodes`. An endpoint
/// qualifies a subgraph `S` when the endpoint is a direct child of `S`
/// (`node.group == Some(S)`) and the edge's LCA is a strict ancestor of `S`
/// (`lca != Some(S)`) — i.e. the edge leaves `S` to reach the rest of the
/// graph, rather than staying internal to `S`.
fn cross_boundary_subs(
    diagram: &Diagram,
    edge_infos: &[EdgeInfo],
    id_index: &std::collections::HashMap<&str, usize>,
) -> std::collections::HashSet<usize> {
    let mut subs = std::collections::HashSet::new();
    for (ei, e) in diagram.edges.iter().enumerate() {
        let Some(info) = edge_infos.get(ei) else { continue; };
        for nid in [e.from.as_str(), e.to.as_str()].map(|id| id_index[id]) {
            if let Some(group) = diagram.nodes[nid].group
                && info.lca != Some(group) {
                    subs.insert(group);
                }
        }
    }
    subs
}

/// `(top_inset, bottom_padding)` for a child subgraph's frame, adding the
/// cross-boundary extra padding ([`CROSS_FRAME_PAD`]) to both when `cross`
/// is true (the subgraph has a cross-boundary edge to an immediate child).
/// Both insets grow along the LCA-level flow axis, where the within-frame
/// stubs jog, so the extra room clears the title (above the node) and the
/// node's arrowhead (below the jog).
fn frame_insets(sg: &Subgraph, cross: bool) -> (f32, f32) {
    let top = if sg.title.is_some() {
        FRAME_TITLE_H
    } else {
        FRAME_PAD_Y
    };
    if cross {
        (top + CROSS_FRAME_PAD, FRAME_PAD_Y + CROSS_FRAME_PAD)
    } else {
        (top, FRAME_PAD_Y)
    }
}

// ============ Cross-boundary edge routing (M5) ============

/// One of the four sides of an axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The center of a rect.
fn center(r: (f32, f32, f32, f32)) -> (f32, f32) {
    (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0)
}

/// The port on `rect`'s `side`, at the rect's own center.
fn port(rect: (f32, f32, f32, f32), side: Side) -> (f32, f32) {
    let (x, y, w, h) = rect;
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    match side {
        Side::Top => (cx, y),
        Side::Bottom => (cx, y + h),
        Side::Left => (x, cy),
        Side::Right => (x + w, cy),
    }
}

/// The point on `rect`'s `side` at the given cross-axis coordinate
/// (clamped to that side's span), rather than at the rect's center. For a
/// vertical flow (top-down / bottom-up) the cross axis is `x` and the sides
/// are [`Side::Top`] / [`Side::Bottom`]; for a horizontal flow it is `y` and
/// the sides are [`Side::Left`] / [`Side::Right`]. Used by [`cross_boundary_path`]
/// to place a subgraph frame's connection point at the *endpoint node's*
/// cross-coordinate (not the frame center), so a within-frame stub runs
/// straight along the node's own axis and edges entering a subgraph line up
/// with their target node rather than all converging on the frame's midpoint.
fn port_at_cross(rect: (f32, f32, f32, f32), side: Side, cross: f32) -> (f32, f32) {
    let (x, y, w, h) = rect;
    match side {
        Side::Top => (cross.clamp(x, x + w), y),
        Side::Bottom => (cross.clamp(x, x + w), y + h),
        Side::Left => (x, cross.clamp(y, y + h)),
        Side::Right => (x + w, cross.clamp(y, y + h)),
    }
}

/// The `x` at which a title-detour should enter a titled frame's top edge so
/// the entry clears the title text, or `None` if no detour is needed.
///
/// A cross-boundary within-frame stub now runs straight at the endpoint
/// node's `x` ([`port_at_cross`]). When that `x` falls under the frame's
/// title text the stub would cross the title; this returns an entry `x` just
/// past the title's right edge (in the clear part of the top band) so the
/// stub can dive in there and jog across to the node below the title. `None`
/// is returned — meaning "stay straight, no detour" — when the node's `x` is
/// already clear of the title, or when the title is so wide there is no room
/// past it inside the frame (the stub then crosses the title, no worse than
/// the previous frame-center design which crossed it at the center).
fn title_detour_clear_x(rep: (f32, f32, f32, f32), node_cross: f32, title_width: f32) -> Option<f32> {
    let (rx, _ry, rw, _rh) = rep;
    let title_x0 = rx + FRAME_TITLE_X;
    let title_x1 = rx + FRAME_TITLE_X + title_width;
    // Node already clear of the title text (to either side)? Stay straight.
    if node_cross < title_x0 - TITLE_CLEAR_GAP || node_cross > title_x1 + TITLE_CLEAR_GAP {
        return None;
    }
    // Enter just past the title's right edge, kept inside the frame.
    let clear_x = (title_x1 + TITLE_CLEAR_GAP).clamp(rx + FRAME_PAD_X, rx + rw - FRAME_PAD_X);
    // If the title is so wide that even the clamped entry is still on/under
    // it, there is no clear entry — give up the detour (fall back to straight).
    if clear_x <= title_x1 + 1e-3 {
        return None;
    }
    Some(clear_x)
}

/// The exit/entry sides for a cross-boundary edge at a level laid out under
/// `dir`, given the page-space centers of its two representatives.
///
/// The sides lie along the level's *direction axis* (vertical for top-down /
/// bottom-up, horizontal for left-right / right-left): an edge between two
/// items at that level always spans ranks along that axis, so its ports are on
/// those sides. The sign — which side is the exit vs. the entry — is read from
/// the representatives' actual positions (which already reflect the direction
/// transform), so it is correct for all four directions. This matches the port
/// the flat engine assigns to any edge at that level.
fn sides_along(dir: Direction, rep_from_c: (f32, f32), rep_to_c: (f32, f32)) -> (Side, Side) {
    match dir {
        Direction::TopDown | Direction::BottomUp => {
            if rep_to_c.1 >= rep_from_c.1 {
                (Side::Bottom, Side::Top)
            } else {
                (Side::Top, Side::Bottom)
            }
        }
        Direction::LeftRight | Direction::RightLeft => {
            if rep_to_c.0 >= rep_from_c.0 {
                (Side::Right, Side::Left)
            } else {
                (Side::Left, Side::Right)
            }
        }
    }
}

/// The point where the segment from `a` to `b` first crosses the boundary of
/// `r` (the smallest `t > 0` with `a + t·(b − a)` on `r`'s perimeter), or
/// `None` if it never does.
///
/// When `a` is inside `r` and `b` outside, this is the *exit* point; when `a`
/// is outside and `b` inside, it is the *entry* point. Used to insert
/// connection points where a cross-boundary stub crosses an intermediate
/// (nested) subgraph frame.
fn line_rect_exit(a: (f32, f32), b: (f32, f32), r: (f32, f32, f32, f32)) -> Option<(f32, f32)> {
    let (rx, ry, rw, rh) = r;
    let (ax, ay) = a;
    let dx = b.0 - ax;
    let dy = b.1 - ay;
    let mut best_t = f32::INFINITY;
    // Vertical walls (left at rx, right at rx+rw).
    if dx.abs() > 1e-9 {
        for wall_x in [rx, rx + rw] {
            let t = (wall_x - ax) / dx;
            if t > 1e-6 && t < best_t {
                let y = ay + t * dy;
                if y >= ry - 1e-4 && y <= ry + rh + 1e-4 {
                    best_t = t;
                }
            }
        }
    }
    // Horizontal walls (top at ry, bottom at ry+rh).
    if dy.abs() > 1e-9 {
        for wall_y in [ry, ry + rh] {
            let t = (wall_y - ay) / dy;
            if t > 1e-6 && t < best_t {
                let x = ax + t * dx;
                if x >= rx - 1e-4 && x <= rx + rw + 1e-4 {
                    best_t = t;
                }
            }
        }
    }
    if best_t.is_finite() {
        Some((ax + best_t * dx, ay + best_t * dy))
    } else {
        None
    }
}

/// Drop consecutive (near-)identical points so the polyline has no zero-length
/// segments.
fn dedup_consecutive(pts: &mut Vec<(f32, f32)>) {
    let mut i = 1;
    while i < pts.len() {
        let (ax, ay) = pts[i - 1];
        let (bx, by) = pts[i];
        if (ax - bx).abs() < 1e-3 && (ay - by).abs() < 1e-3 {
            pts.remove(i);
        } else {
            i += 1;
        }
    }
}

/// Does segment `p0`→`p1` intersect the closed axis-aligned rect `r`?
/// (Liang–Barsky line clipping.) Used to detect when a cross-boundary
/// within-frame stub would pierce a sibling, and to reject an around-frame
/// route whose perpendicular entry would cut across a sibling.
fn segment_intersects_rect(
    p0: (f32, f32),
    p1: (f32, f32),
    r: (f32, f32, f32, f32),
) -> bool {
    let (x0, y0) = p0;
    let (x1, y1) = p1;
    let (rx, ry, rw, rh) = r;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    for (p, q) in [
        (-dx, x0 - rx),
        (dx, rx + rw - x0),
        (-dy, y0 - ry),
        (dy, ry + rh - y0),
    ] {
        if p.abs() < 1e-9 {
            if q < -1e-9 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return false;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return false;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    t0 < t1 - 1e-6
}

// ============ Orthogonal edge routing (M7) ============

/// Which page-space axis an edge flows along — the rank axis after the
/// direction transform. Vertical for top-down / bottom-up, horizontal for
/// left-right / right-left. [`orthogonalize`] uses it to orient bends along
/// the flow axis so the perpendicular jog lands between ranks (or in a frame's
/// padding), clear of node interiors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowAxis {
    /// Flow along y (top-down / bottom-up).
    Vertical,
    /// Flow along x (left-right / right-left).
    Horizontal,
}

impl FlowAxis {
    fn from_direction(dir: Direction) -> Self {
        match dir {
            Direction::TopDown | Direction::BottomUp => FlowAxis::Vertical,
            Direction::LeftRight | Direction::RightLeft => FlowAxis::Horizontal,
        }
    }
}

/// `(cross, flow)` coordinates of `p` along `axis`:
/// - [`FlowAxis::Vertical`]: cross = x, flow = y.
/// - [`FlowAxis::Horizontal`]: cross = y, flow = x.
fn cross_flow(p: (f32, f32), axis: FlowAxis) -> (f32, f32) {
    match axis {
        FlowAxis::Vertical => (p.0, p.1),
        FlowAxis::Horizontal => (p.1, p.0),
    }
}

/// Rebuild a point from its cross coordinate and a new flow coordinate.
fn with_flow(cross: f32, flow: f32, axis: FlowAxis) -> (f32, f32) {
    match axis {
        FlowAxis::Vertical => (cross, flow),
        FlowAxis::Horizontal => (flow, cross),
    }
}

/// The `(low, high)` cross-axis span of a rect along `axis` (the axis
/// perpendicular to the flow): the x-span for a vertical flow, the y-span for
/// a horizontal flow.
fn cross_range(r: (f32, f32, f32, f32), axis: FlowAxis) -> (f32, f32) {
    match axis {
        FlowAxis::Vertical => (r.0, r.0 + r.2),
        FlowAxis::Horizontal => (r.1, r.1 + r.3),
    }
}

/// The `(low, high)` flow-axis span of a rect along `axis`: the y-span for a
/// vertical flow, the x-span for a horizontal flow.
fn flow_range(r: (f32, f32, f32, f32), axis: FlowAxis) -> (f32, f32) {
    match axis {
        FlowAxis::Vertical => (r.1, r.1 + r.3),
        FlowAxis::Horizontal => (r.0, r.0 + r.2),
    }
}

/// Are two rects (near-)identical? Used to exclude the target node itself
/// when checking its frame's other children for a stub pierce.
fn rects_near(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    (a.0 - b.0).abs() < 1e-2
        && (a.1 - b.1).abs() < 1e-2
        && (a.2 - b.2).abs() < 1e-2
        && (a.3 - b.3).abs() < 1e-2
}

/// The flow-coordinate at which to jog the segment `p0`→`p1` (in `(cross,
/// flow)` form): at the segment midpoint, where the perpendicular jog of an
/// orthogonalized "Z" lands in an inter-rank gap or a frame's padding (clear
/// of node interiors and titles). A cross-boundary within-frame stub that
/// would cross a titled frame's TOP side instead places its jog itself (via
/// [`title_detour`], a fixed [`STUB_JOG_CLEARANCE`] above the node) before
/// orthogonalization, so the midpoint here never lands on a title.
fn jog_flow(f0: f32, f1: f32) -> f32 {
    (f0 + f1) / 2.0
}

/// Orthogonalize a polyline chain: each diagonal segment becomes a right-
/// angle "Z" (flow to the segment's flow midpoint, across to the target's
/// cross coordinate, then flow to the target). Segments already aligned
/// (along either axis) are left straight; consecutive duplicate points are
/// dropped.
///
/// The existing waypoints are all preserved: they sit on node ports, dummy
/// centers, and frame connection points that the M5 tests assert on, and a
/// long edge whose dummies line up under its source is still drawn through
/// those dummies (collinear extra points render the same straight line).
fn ortho_chain(points: &[(f32, f32)], axis: FlowAxis) -> Vec<(f32, f32)> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len() * 2);
    out.push(points[0]);
    for w in points.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let (c0, f0) = cross_flow(p0, axis);
        let (c1, f1) = cross_flow(p1, axis);
        if (c0 - c1).abs() < 1e-3 || (f0 - f1).abs() < 1e-3 {
            // Already axis-aligned along the flow or cross axis: keep straight.
            out.push(p1);
        } else {
            let jf = jog_flow(f0, f1);
            out.push(with_flow(c0, jf, axis));
            out.push(with_flow(c1, jf, axis));
            out.push(p1);
        }
    }
    dedup_consecutive(&mut out);
    out
}

/// Orthogonalize along the segment midpoints — the default for direct edges,
/// whose jogs land in inter-rank gaps (clear of nodes and titles), and for the
/// pre-aligned (already axis-aligned) detour stubs of cross-boundary edges.
fn orthogonalize(points: &[(f32, f32)], axis: FlowAxis) -> Vec<(f32, f32)> {
    ortho_chain(points, axis)
}

/// A clean "around the target frame" route for a cross-boundary edge whose
/// straight within-frame target stub would pierce the target frame's internal
/// content — the target node sits beyond other members along the frame's
/// internal flow axis (e.g. a top-down Storage subgraph entered from the top
/// to reach its bottom-most Database node, with Cache in between). The
/// straight stub would run straight through Cache and then exactly overlap
/// the Cache→Database edge.
///
/// Instead the edge routes from the source node, around the *outside* of the
/// target frame, and into the target node from a side perpendicular to the
/// flow axis, so it clears the frame's other members and stays distinct from
/// any internal edge it would otherwise have overlapped. The groups' internal
/// layouts are untouched (the M5 headline guarantee — the failure mode this
/// tool exists to escape).
///
/// Returns the full path (source node port → around the frame → target node
/// port), already axis-aligned, or `None` if a clean route is not available —
/// a sibling blocks the perpendicular entry on both sides — in which case the
/// caller falls back to the straight stub. Only the single-target-frame,
/// single-or-zero-source-frame case is handled; deeper nesting falls back
/// too.
#[allow(clippy::too_many_arguments)]
fn try_around_target_route(
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    fb: (f32, f32, f32, f32),
    from_chain: &[usize],
    axis: FlowAxis,
    exit: Side,
    entry: Side,
    siblings: &[(f32, f32, f32, f32)],
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
) -> Option<Vec<(f32, f32)>> {
    let from_port = port(from, exit);
    let (from_cc, from_ff) = cross_flow(from_port, axis);
    let (fb_clo, fb_chi) = cross_range(fb, axis);
    let (to_clo, to_chi) = cross_range(to, axis);
    let to_fc = {
        let (lo, hi) = flow_range(to, axis);
        (lo + hi) / 2.0
    };
    let fb_cc = (fb_clo + fb_chi) / 2.0;

    // Build the around-route for a given perpendicular side. `around_hi` is
    // the high-cross side — Right for a vertical flow, Bottom for a horizontal
    // flow. The route is built in `(cross, flow)` space and mapped back to
    // `(x, y)` at the end.
    let build = |around_hi: bool| -> Vec<(f32, f32)> {
        let ac = if around_hi { fb_chi } else { fb_clo };
        let node_ac = if around_hi { to_chi } else { to_clo };
        // Is the source node already clear of the frame on this side (its
        // cross-coordinate lies outside the frame's cross-span)? If so the
        // route can run straight along the flow axis to the target's flow-
        // coordinate then across into the frame — a clean L. Otherwise the
        // source sits above the frame within its cross-span: drop into the
        // gap between the source and target frames, cross to the around-
        // side, then run down it to the target's flow-coordinate.
        let outside = if around_hi { from_cc > fb_chi } else { from_cc < fb_clo };
        let mut cf: Vec<(f32, f32)> = vec![(from_cc, from_ff)];
        if outside {
            cf.push((from_cc, to_fc));
            cf.push((ac, to_fc));
        } else {
            let src_exit_flow = if from_chain.is_empty() {
                from_ff
            } else {
                cross_flow(port(frame_rect(*from_chain.last().unwrap()), exit), axis).1
            };
            let fb_entry_flow = cross_flow(port(fb, entry), axis).1;
            let lo = src_exit_flow.min(fb_entry_flow);
            let hi = src_exit_flow.max(fb_entry_flow);
            let gap_f = ((src_exit_flow + fb_entry_flow) / 2.0).clamp(lo, hi);
            cf.push((from_cc, gap_f));
            cf.push((ac, gap_f));
            cf.push((ac, to_fc));
        }
        cf.push((node_ac, to_fc));
        cf.into_iter().map(|(c, f)| with_flow(c, f, axis)).collect()
    };

    // Prefer the around-side toward the source (shorter, no backtracking);
    // fall back to the far side if a sibling blocks the near perpendicular
    // entry. Reject a candidate only if one of its segments actually crosses
    // a sibling — the around-route otherwise stays outside the frame, so this
    // only fires for a perpendicular entry that cuts across a sibling at the
    // target's flow-coordinate.
    let near_hi = from_cc >= fb_cc;
    for around_hi in [near_hi, !near_hi] {
        let route = build(around_hi);
        let blocked = route.windows(2).any(|w| {
            siblings
                .iter()
                .any(|&sib| segment_intersects_rect(w[0], w[1], sib))
        });
        if !blocked {
            return Some(route);
        }
    }
    None
}

/// Frame-aware path for a cross-boundary edge from `from` to `to` (absolute
/// node rects).
///
/// `from_chain` / `to_chain` list the subgraph frames containing each endpoint,
/// innermost first, ending with the representative at the LCA level (the
/// outermost frame the edge crosses on that side). An empty chain means the
/// endpoint is a direct child of the LCA, so the endpoint node *is* its own
/// representative. `frame_rect(i)` returns the absolute frame rect;
/// `frame_title_width(i)` returns the rendered width of that subgraph's title
/// (or `None` if it has none).
///
/// The path runs:
/// `from`-port → [intermediate source frame crossings] → source-rep port →
/// target-rep port → [intermediate target frame crossings] → `to`-port.
/// The middle (rep-port → rep-port) is the LCA-level segment between the two
/// representatives' ports. Each rep port sits at the **endpoint node's**
/// cross-coordinate on its frame's facing side ([`port_at_cross`]) — not the
/// frame's center — so a within-frame stub runs straight along the node's own
/// axis and edges entering a subgraph line up with their target node instead
/// of all converging on the frame's midpoint (which read as every edge
/// reaching every member). Intermediate nested frames are clipped at their
/// boundaries.
///
/// Title avoidance: a straight stub at the node's cross-coordinate would
/// cross the frame's title text when the node sits under the (top-left)
/// title. In the common single-frame, top-down case — where the frame is grown
/// ([`CROSS_FRAME_PAD`]) so there is a clear band below the title — the stub
/// instead enters the frame just past the title's right edge
/// ([`title_detour_clear_x`]) and jogs across to the node below the title
/// ([`STUB_JOG_CLEARANCE`] above the node), keeping the title clear. When the
/// node is already clear of the title (the usual case for a narrow title) the
/// stub stays straight; deeper nesting, or a title too wide to clear, keeps
/// the straight stub (crossing the title no worse than the prior frame-center
/// design). The returned (possibly diagonal) segments are turned into
/// right-angle bends by [`ortho_chain`] (M7).
#[allow(clippy::too_many_arguments)]
fn cross_boundary_path(
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    from_chain: &[usize],
    to_chain: &[usize],
    lca_dir: Direction,
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
    frame_title_width: &dyn Fn(usize) -> Option<f32>,
    frame_children: &dyn Fn(usize) -> Vec<(f32, f32, f32, f32)>,
) -> Vec<(f32, f32)> {
    let axis = FlowAxis::from_direction(lca_dir);
    let rep_from = if let Some(&s) = from_chain.last() {
        frame_rect(s)
    } else {
        from
    };
    let rep_to = if let Some(&s) = to_chain.last() {
        frame_rect(s)
    } else {
        to
    };
    let (exit, entry) = sides_along(lca_dir, center(rep_from), center(rep_to));
    let from_port = port(from, exit);
    let to_port = port(to, entry);
    // Node-aligned rep ports: at each endpoint node's cross-coordinate on its
    // rep frame's facing side. (When the chain is empty the endpoint is its
    // own rep, so this is just the node's own facing port.) This is what makes
    // a within-frame stub straight and lets edges enter a subgraph lined up
    // with their target node.
    let from_cross = cross_flow(center(from), axis).0;
    let to_cross = cross_flow(center(to), axis).0;
    let rep_from_port_aligned = port_at_cross(rep_from, exit, from_cross);
    let rep_to_port_aligned = port_at_cross(rep_to, entry, to_cross);

    // If the straight within-frame target stub would pierce the target
    // frame's internal content (the target node sits beyond other members
    // along the frame's internal flow axis — e.g. a top-down subgraph entered
    // from the top to reach its bottom-most node), route around the frame and
    // into the target from a perpendicular side instead of running straight to
    // the node. See [`try_around_target_route`]. The pierce test uses the
    // node-aligned rep port (the straight stub at the node's cross-coordinate);
    // only the common single-target-frame, single-or-zero-source-frame case is
    // handled here; deeper nesting keeps the straight stub.
    if to_chain.len() == 1 && from_chain.len() <= 1 {
        let sibs: Vec<(f32, f32, f32, f32)> = frame_children(to_chain[0])
            .into_iter()
            .filter(|r| !rects_near(*r, to))
            .collect();
        let target_pierced = sibs
            .iter()
            .any(|&sib| segment_intersects_rect(rep_to_port_aligned, to_port, sib));
        if target_pierced
            && let Some(route) = try_around_target_route(
                from,
                to,
                rep_to,
                from_chain,
                axis,
                exit,
                entry,
                &sibs,
                frame_rect,
            )
        {
            return route;
        }
    }

    // Title detours: the Top side of a titled, single-frame (grown) rep under
    // top-down has a clear band below the title where the stub can jog. Each
    // returns the clear entry cross-coordinate past the title; the rep port
    // then moves there and the stub jogs across to the node below the title.
    // `None` keeps the straight, node-aligned stub.
    let from_detour = title_detour(rep_from, from, from_chain, exit, lca_dir, frame_title_width);
    let to_detour = title_detour(rep_to, to, to_chain, entry, lca_dir, frame_title_width);
    let rep_from_port = match from_detour {
        Some(d) => port_at_cross(rep_from, exit, d.clear_cross),
        None => rep_from_port_aligned,
    };
    let rep_to_port = match to_detour {
        Some(d) => port_at_cross(rep_to, entry, d.clear_cross),
        None => rep_to_port_aligned,
    };

    let mut pts = Vec::new();

    // Source stub: the endpoint node's port -> its representative's frame
    // port, clipping at any intermediate (nested) source frame boundaries.
    if !from_chain.is_empty() {
        let stub = build_stub(
            from_port,
            rep_from_port,
            from_cross,
            from_chain,
            from_detour,
            axis,
            frame_rect,
            /* inward */ false,
        );
        pts.extend(ortho_chain(&stub, axis));
    }

    // LCA-level segment between the two representatives' ports, jogged at the
    // midpoint (the inter-representative gap — clear of nodes and titles).
    pts.extend(ortho_chain(&[rep_from_port, rep_to_port], axis));

    // Target stub: symmetric to the source stub.
    if !to_chain.is_empty() {
        let stub = build_stub(
            to_port,
            rep_to_port,
            to_cross,
            to_chain,
            to_detour,
            axis,
            frame_rect,
            /* inward */ true,
        );
        pts.extend(ortho_chain(&stub, axis));
    }

    dedup_consecutive(&mut pts);
    pts
}

/// A title detour for a within-frame stub: the cross-coordinate at which to
/// enter/leave the frame (just past the title text) and the flow-coordinate
/// of the below-title jog (just above the node). Both are in the page-space
/// axis convention of [`cross_boundary_path`] (cross = `x`, flow = `y` for
/// the top-down / Top-side case that is the only one a detour arises for).
#[derive(Clone, Copy)]
struct TitleDetour {
    clear_cross: f32,
    jog_flow: f32,
}

/// Whether a within-frame stub on `side` of `rep` needs a title detour, for
/// an endpoint `node` whose container chain is `chain`. See
/// [`title_detour_clear_x`] for the geometry. A detour only arises on the Top
/// side (where the title lives), under top-down, for a single-frame chain —
/// the case where the frame is grown ([`CROSS_FRAME_PAD`]) so there is a
/// clear band below the title for the jog.
fn title_detour(
    rep: (f32, f32, f32, f32),
    node: (f32, f32, f32, f32),
    chain: &[usize],
    side: Side,
    lca_dir: Direction,
    frame_title_width: &dyn Fn(usize) -> Option<f32>,
) -> Option<TitleDetour> {
    if side != Side::Top || lca_dir != Direction::TopDown || chain.len() != 1 {
        return None;
    }
    let rep_idx = *chain.last()?;
    let title_w = frame_title_width(rep_idx)?;
    // The Top side is a vertical flow: cross = x, so the node's cross-
    // coordinate is its center x, and the jog sits just above its top.
    let node_cross = center(node).0;
    let clear_cross = title_detour_clear_x(rep, node_cross, title_w)?;
    let jog_flow = node.1 - STUB_JOG_CLEARANCE;
    Some(TitleDetour { clear_cross, jog_flow })
}

/// Build a within-frame stub between `node_port` and `rep_port`, clipping at
/// intermediate (nested) frame boundaries, with a title detour at the rep end
/// when `detour` is set. `inward` is `true` for a target stub (rep port →
/// node port) and `false` for a source stub (node port → rep port); the detour
/// and the crossings are ordered accordingly. A detour only arises for a
/// single-frame chain (no intermediate frames), so crossings and the detour
/// never co-occur.
#[allow(clippy::too_many_arguments)]
fn build_stub(
    node_port: (f32, f32),
    rep_port: (f32, f32),
    node_cross: f32,
    chain: &[usize],
    detour: Option<TitleDetour>,
    axis: FlowAxis,
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
    inward: bool,
) -> Vec<(f32, f32)> {
    let Some(d) = detour else {
        // No detour: node-aligned straight stub (both ends at the node's
        // cross-coordinate), clipping at each intermediate frame boundary.
        let mut stub = Vec::new();
        if inward {
            stub.push(rep_port);
            for &s in chain[..chain.len() - 1].iter().rev() {
                if let Some(p) = line_rect_exit(rep_port, node_port, frame_rect(s)) {
                    stub.push(p);
                }
            }
            stub.push(node_port);
        } else {
            stub.push(node_port);
            for &s in &chain[..chain.len() - 1] {
                if let Some(p) = line_rect_exit(node_port, rep_port, frame_rect(s)) {
                    stub.push(p);
                }
            }
            stub.push(rep_port);
        }
        return stub;
    };
    // Title detour: enter/leave the frame at the clear cross-coordinate past
    // the title, jog across to the node's cross-coordinate at `jog_flow`
    // (just below the title), then continue to the node. Single-frame, so no
    // intermediate frames to clip.
    let jog_clear = with_flow(d.clear_cross, d.jog_flow, axis);
    let jog_node = with_flow(node_cross, d.jog_flow, axis);
    if inward {
        vec![rep_port, jog_clear, jog_node, node_port]
    } else {
        vec![node_port, jog_node, jog_clear, rep_port]
    }
}

// ============ The flat engine ============

/// Lay out a single flat graph: `items` with fixed sizes, `edges` between
/// item indices (original arrow direction), under `direction`, with the
/// given page `margin`. Returns per-item rects and per-edge waypoints in the
/// requested direction's final orientation, plus canvas size.
fn layout_flat(
    items: &[FlatItem],
    edges: &[FlatEdge],
    direction: Direction,
    margin: f32,
) -> FlatResult {
    if items.is_empty() {
        return FlatResult {
            rects: Vec::new(),
            edges: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    }

    let mut nodes: Vec<LNode> = items
        .iter()
        .map(|it| LNode {
            w: it.w,
            h: it.h,
            rank: 0,
        })
        .collect();
    let n = nodes.len();

    let mut orig_edges: Vec<OrigEdge> = Vec::with_capacity(edges.len());
    let mut dag_edges: Vec<(usize, usize)> = Vec::new();
    remove_cycles(edges, n, &mut orig_edges, &mut dag_edges);

    longest_path(&mut nodes, &dag_edges);

    let layer_edges = insert_dummies(&mut nodes, &mut orig_edges);

    let max_rank = nodes.iter().map(|nd| nd.rank).max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (i, nd) in nodes.iter().enumerate() {
        layers[nd.rank].push(i);
    }

    let mut upper_neighbors: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut lower_neighbors: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for &(u, v) in &layer_edges {
        lower_neighbors[u].push(v);
        upper_neighbors[v].push(u);
    }

    let mut order = vec![0usize; nodes.len()];
    for layer in layers.iter() {
        for (i, &v) in layer.iter().enumerate() {
            order[v] = i;
        }
    }
    minimize_crossings(
        &mut layers,
        &mut order,
        &layer_edges,
        &upper_neighbors,
        &lower_neighbors,
        &nodes,
    );

    // The drawn box is always the text-fitted, wide-and-short size `(dw, dh)`
    // — labels are never rotated, so the box must always fit the horizontal
    // text. Only the box's *position* is transformed by the direction.
    //
    // For the canonical (top-down) layout computation, though, the wide drawn
    // dimension lies along the *rank* axis in horizontal directions, not the
    // order axis. So each node has a layout footprint `(lw, lh)` in canonical
    // space (x = order, y = rank): for vertical directions it equals the
    // drawn box; for horizontal directions it is swapped, so that spacing
    // along the order axis uses the short dimension and layer bands along the
    // rank axis use the wide dimension. Using `(lw, lh)` for spacing and port
    // math (and `(dw, dh)` only for the final rendered rect) keeps boxes
    // wide-short in every direction while keeping ports glued to the correct
    // side.
    let dw: Vec<f32> = nodes.iter().map(|nd| nd.w).collect();
    let dh: Vec<f32> = nodes.iter().map(|nd| nd.h).collect();
    let (lw, lh): (Vec<f32>, Vec<f32>) = match direction {
        Direction::TopDown | Direction::BottomUp => (dw.clone(), dh.clone()),
        Direction::LeftRight | Direction::RightLeft => (dh.clone(), dw.clone()),
    };
    let x = assign_x(&layers, &upper_neighbors, &lw);
    let y = assign_y(&layers, &lh);

    let (x, y, canon_w, canon_h) = normalize(&x, &y, &lw, &lh, margin);

    let mut rects = Vec::with_capacity(items.len());
    for i in 0..items.len() {
        let (tx, ty) = transform_pos((x[i], y[i], lh[i]), direction, canon_h);
        // Drawn at the true text-fitted size, not the (possibly swapped)
        // layout footprint.
        rects.push((tx, ty, dw[i], dh[i]));
    }

    let mut edges_out = Vec::with_capacity(orig_edges.len());
    for e in &orig_edges {
        let pts = edge_waypoints(e, &x, &y, &lw, &lh);
        let tpts: Vec<(f32, f32)> = pts
            .iter()
            .map(|&(px, py)| transform_point((px, py), direction, canon_h))
            .collect();
        edges_out.push(FlatEdgeOut {
            from: e.from,
            to: e.to,
            points: tpts,
        });
    }

    let (width, height) = target_dims(canon_w, canon_h, direction);
    FlatResult {
        rects,
        edges: edges_out,
        width,
        height,
    }
}

// ============ Phase 1: cycle removal (DFS) ============

/// Build `orig_edges` (with DAG orientation) and `dag_edges` (acyclic) from
/// the flat edge list.
///
/// A standard DFS over the *original* edge directions detects back-edges
/// (edges to a node currently on the recursion stack); those are reversed in
/// the DAG. The reversal is recorded per original edge so the output arrow
/// still points `from` → `to`.
fn remove_cycles(
    edges: &[FlatEdge],
    n: usize,
    orig_edges: &mut Vec<OrigEdge>,
    dag_edges: &mut Vec<(usize, usize)>,
) {
    // Original outgoing adjacency: (target, original edge index), in edge-index
    // order so the DFS traversal is deterministic and reproducible.
    let mut out_adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (ei, e) in edges.iter().enumerate() {
        out_adj[e.from].push((e.to, ei));
    }

    let mut by_edge: Vec<OrigEdge> = (0..edges.len())
        .map(|_| OrigEdge {
            from: usize::MAX,
            to: usize::MAX,
            reversed: false,
            dag_low: usize::MAX,
            dag_high: usize::MAX,
            path: Vec::new(),
        })
        .collect();

    // 0 = unvisited, 1 = on stack (gray), 2 = done (black). Iterative DFS so
    // large graphs can't overflow the native stack.
    let mut state = vec![0u8; n];
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = Vec::new(); // (node, next edge idx to process)
        state[start] = 1;
        stack.push((start, 0));
        while let Some(&(u, ei)) = stack.last() {
            if ei >= out_adj[u].len() {
                state[u] = 2;
                stack.pop();
                continue;
            }
            stack.last_mut().unwrap().1 = ei + 1;
            let (v, edge_idx) = out_adj[u][ei];
            match state[v] {
                0 => {
                    by_edge[edge_idx] = OrigEdge {
                        from: u,
                        to: v,
                        reversed: false,
                        dag_low: u,
                        dag_high: v,
                        path: Vec::new(),
                    };
                    dag_edges.push((u, v));
                    state[v] = 1;
                    stack.push((v, 0));
                }
                1 => {
                    // Back edge: reverse it in the DAG, but remember the flip
                    // so the output arrow still runs `from` -> `to`.
                    by_edge[edge_idx] = OrigEdge {
                        from: u,
                        to: v,
                        reversed: true,
                        dag_low: v,
                        dag_high: u,
                        path: Vec::new(),
                    };
                    dag_edges.push((v, u));
                }
                2 => {
                    // Forward / cross edge: consistent with the topo order.
                    by_edge[edge_idx] = OrigEdge {
                        from: u,
                        to: v,
                        reversed: false,
                        dag_low: u,
                        dag_high: v,
                        path: Vec::new(),
                    };
                    dag_edges.push((u, v));
                }
                _ => unreachable!(),
            }
        }
    }
    orig_edges.extend(by_edge);
}

// ============ Phase 2: layering (longest path) ============

/// Assign `rank` via longest path (Kahn's topological order + relaxation).
/// Sources get rank 0; every DAG edge (u,v) satisfies `rank[v] > rank[u]`.
fn longest_path(nodes: &mut [LNode], dag_edges: &[(usize, usize)]) {
    let n = nodes.len();
    let mut out_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(u, v) in dag_edges {
        if u == v {
            continue; // self-loops don't participate in ranking
        }
        out_adj[u].push(v);
        indeg[v] += 1;
    }
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut processed = 0usize;
    while let Some(u) = queue.pop_front() {
        processed += 1;
        for &v in &out_adj[u] {
            nodes[v].rank = nodes[v].rank.max(nodes[u].rank + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    // If a cycle slipped through (it must not, post cycle-removal), the
    // remaining nodes keep rank 0 — they will still render, just stacked.
    debug_assert_eq!(processed, n, "cycle remained after cycle removal");
}

// ============ Phase 3a: dummy insertion ============

/// For every original edge spanning more than one rank, insert zero-size
/// dummy nodes at the intermediate ranks. Returns the list of adjacent-rank
/// edges (real/dummy → real/dummy). Also fills `path` on each original edge.
fn insert_dummies(nodes: &mut Vec<LNode>, orig_edges: &mut [OrigEdge]) -> Vec<(usize, usize)> {
    let mut layer_edges: Vec<(usize, usize)> = Vec::new();
    for e in orig_edges.iter_mut() {
        if e.dag_low == usize::MAX {
            // Unresolved edge (shouldn't happen); leave a degenerate path.
            e.path = Vec::new();
            continue;
        }
        if e.from == e.to {
            // Self-loop: no dummies, no layer edge. A tiny stub is emitted at
            // waypoint time. Routing is improved in milestone 7.
            e.path = vec![e.from];
            continue;
        }
        let lo = e.dag_low;
        let hi = e.dag_high;
        let mut path = vec![lo];
        let mut prev = lo;
        for r in (nodes[lo].rank + 1)..nodes[hi].rank {
            let d = nodes.len();
            nodes.push(LNode {
                w: 0.0,
                h: 0.0,
                rank: r,
            });
            path.push(d);
            layer_edges.push((prev, d));
            prev = d;
        }
        path.push(hi);
        layer_edges.push((prev, hi));
        e.path = path;
    }
    layer_edges
}

// ============ Phase 3b: crossing minimization (barycenter) ============

fn minimize_crossings(
    layers: &mut Vec<Vec<usize>>,
    order: &mut [usize],
    layer_edges: &[(usize, usize)],
    upper_neighbors: &[Vec<usize>],
    lower_neighbors: &[Vec<usize>],
    nodes: &[LNode],
) {
    // Group layer edges by the rank of their upper endpoint for crossing count.
    let mut edges_by_rank: Vec<Vec<(usize, usize)>> = vec![Vec::new(); layers.len()];
    for &(u, v) in layer_edges {
        edges_by_rank[nodes[u].rank].push((u, v));
    }

    let mut best_cross = count_crossings(order, &edges_by_rank);
    let mut best_layers = layers.clone();

    for _ in 0..CROSS_ITERS {
        // Downward sweep: reorder layer r using upper neighbors in r-1.
        for r in 1..layers.len() {
            reorder_by_bary(r, true, layers, order, upper_neighbors, lower_neighbors);
        }
        // Upward sweep: reorder layer r using lower neighbors in r+1.
        for r in (0..layers.len() - 1).rev() {
            reorder_by_bary(r, false, layers, order, upper_neighbors, lower_neighbors);
        }
        let c = count_crossings(order, &edges_by_rank);
        if c < best_cross {
            best_cross = c;
            best_layers = layers.clone();
        }
        if best_cross == 0 {
            break;
        }
    }

    *layers = best_layers;
    // Refresh `order` from the chosen layers.
    for layer in layers.iter() {
        for (i, &v) in layer.iter().enumerate() {
            order[v] = i;
        }
    }
}

/// Reorder `layers[r]` by the barycenter of the relevant neighbors' positions,
/// preserving relative order for ties (stable).
fn reorder_by_bary(
    r: usize,
    use_upper: bool,
    layers: &mut [Vec<usize>],
    order: &mut [usize],
    upper_neighbors: &[Vec<usize>],
    lower_neighbors: &[Vec<usize>],
) {
    let layer = std::mem::take(&mut layers[r]);
    // (node, barycenter, current position)
    let mut keyed: Vec<(usize, f32, usize)> =
        Vec::with_capacity(layer.len());
    for (i, &v) in layer.iter().enumerate() {
        let neigh = if use_upper {
            &upper_neighbors[v]
        } else {
            &lower_neighbors[v]
        };
        let bary = if neigh.is_empty() {
            order[v] as f32
        } else {
            neigh.iter().map(|&u| order[u] as f32).sum::<f32>() / neigh.len() as f32
        };
        keyed.push((v, bary, i));
    }
    keyed.sort_by(|a, b| {
        a.1
            .partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.2.cmp(&b.2))
    });
    layers[r] = keyed.iter().map(|&(v, _, _)| v).collect();
    for (i, &(v, _, _)) in keyed.iter().enumerate() {
        order[v] = i;
    }
}

/// Count edge crossings across all adjacent layer pairs.
fn count_crossings(order: &[usize], edges_by_rank: &[Vec<(usize, usize)>]) -> usize {
    let mut total = 0usize;
    for edges in edges_by_rank {
        for i in 0..edges.len() {
            let (a, b) = edges[i];
            for &(c, d) in &edges[i + 1..] {
                if a == c || b == d {
                    continue; // share an endpoint → no crossing
                }
                let (oa, ob, oc, od) = (order[a], order[b], order[c], order[d]);
                if (oa < oc) != (ob < od) {
                    total += 1;
                }
            }
        }
    }
    total
}

// ============ Phase 4a: x-coordinate assignment ============

/// Centered block coordinate assignment. Within a rank, nodes keep their
/// left-packed spread (so siblings sit side by side with `NODE_SEP` gaps);
/// each non-source rank is then translated as a block so its center lands on
/// the centroid of its distinct parents' centers. Nodes never swap order
/// (crossing minimization already fixed it).
///
/// This is what makes a chain line up vertically and a rank that shares one
/// parent center as a group beneath it (milestone 5.5): a chain of singleton
/// ranks collapses onto a single axis, a single-parent rank centers under
/// that parent, and a rank fanning into one child centers on it. Parents are
/// finalized before children (top-down), and a per-rank uniform shift
/// preserves within-rank order and gaps (no overlaps introduced); ranks are
/// stacked along the rank axis, so cross-rank overlap along x is intentional
/// and needs no collision check.
fn assign_x(layers: &[Vec<usize>], upper_neighbors: &[Vec<usize>], w: &[f32]) -> Vec<f32> {
    let n = w.len();
    let mut x = vec![0.0_f32; n];
    // Initialize: left-pack each layer by current order. This fixes the
    // within-rank spread (relative positions + gaps); the centering pass
    // below only translates each rank as a whole.
    for layer in layers.iter() {
        let mut cx = 0.0;
        for &v in layer.iter() {
            x[v] = cx;
            cx += w[v] + NODE_SEP;
        }
    }
    center_blocks(layers, upper_neighbors, w, &mut x);
    x
}

/// Translate each non-source rank so its block center matches the centroid
/// of its distinct parents' centers. See [`assign_x`].
///
/// "Parent" here is the *real* ancestor behind an upper neighbor: a long
/// edge is split into zero-size dummy nodes, and a dummy's upper neighbor is
/// another dummy or the real low-rank endpoint. We walk up that chain so a
/// node reached only via a long edge still centers under the real node that
/// originates it, rather than under an intermediate dummy.
fn center_blocks(layers: &[Vec<usize>], upper_neighbors: &[Vec<usize>], w: &[f32], x: &mut [f32]) {
    for layer in layers.iter() {
        if layer.is_empty() {
            continue;
        }
        // Distinct real parents of this rank's members (deterministic order).
        let mut parents: Vec<usize> = Vec::new();
        for &v in layer {
            for &u in &upper_neighbors[v] {
                let ra = real_ancestor(u, upper_neighbors, w);
                if !parents.contains(&ra) {
                    parents.push(ra);
                }
            }
        }
        if parents.is_empty() {
            continue; // source rank: keep left-packed init
        }
        let anchor = parents
            .iter()
            .map(|&u| x[u] + w[u] / 2.0)
            .sum::<f32>()
            / parents.len() as f32;

        // Block span over real (non-dummy) nodes so zero-size dummies on long
        // edges don't skew the center; fall back to all if the rank is purely
        // dummies.
        let real: Vec<usize> = layer.iter().copied().filter(|&v| w[v] > 0.0).collect();
        let span: &[usize] = if real.is_empty() {
            layer.as_slice()
        } else {
            real.as_slice()
        };
        let min_left = span.iter().map(|&v| x[v]).fold(f32::INFINITY, f32::min);
        let max_right = span
            .iter()
            .map(|&v| x[v] + w[v])
            .fold(f32::NEG_INFINITY, f32::max);
        let block_center = (min_left + max_right) / 2.0;
        let shift = anchor - block_center;
        if shift.abs() > 1e-4 {
            for &v in layer {
                x[v] += shift;
            }
        }
    }
}

/// Walk up the upper-neighbor chain from `u` until a real (non-dummy,
/// `w > 0`) node is reached. Dummies sit on long edges as a linear chain
/// (each has exactly one upper neighbor — the previous dummy or the real
/// low-rank endpoint), so this terminates at the real node that originates
/// the edge.
fn real_ancestor(mut u: usize, upper_neighbors: &[Vec<usize>], w: &[f32]) -> usize {
    while w[u] == 0.0 {
        match upper_neighbors[u].first() {
            Some(&p) => u = p,
            None => break, // malformed dummy; leave as-is
        }
    }
    u
}

// ============ Phase 4b: y-coordinate assignment ============

/// Stack ranks top-to-bottom; each rank is a band of height `max(node h)`.
/// Nodes are centered vertically within their band.
fn assign_y(layers: &[Vec<usize>], h: &[f32]) -> Vec<f32> {
    let n = h.len();
    let mut y = vec![0.0_f32; n];
    let mut rank_y = 0.0_f32;
    for layer in layers.iter() {
        let band = layer
            .iter()
            .map(|&v| h[v])
            .fold(0.0_f32, f32::max);
        for &v in layer.iter() {
            y[v] = rank_y + (band - h[v]) / 2.0;
        }
        rank_y += band + RANK_GAP;
    }
    y
}

// ============ Normalization + direction transform ============

/// Shift so the drawing's top-left content corner sits at `margin`, and return
/// the canonical (top-down) canvas size with a margin on all sides.
fn normalize(
    x: &[f32],
    y: &[f32],
    w: &[f32],
    h: &[f32],
    margin: f32,
) -> (Vec<f32>, Vec<f32>, f32, f32) {
    let n = x.len();
    let mut nx = vec![0.0_f32; n];
    let mut ny = vec![0.0_f32; n];
    let min_x = x.iter().copied().fold(f32::INFINITY, f32::min);
    let min_y = y.iter().copied().fold(f32::INFINITY, f32::min);
    for i in 0..n {
        nx[i] = x[i] - min_x + margin;
        ny[i] = y[i] - min_y + margin;
    }
    let max_right = (0..n)
        .map(|i| nx[i] + w[i])
        .fold(0.0_f32, f32::max);
    let max_bottom = (0..n)
        .map(|i| ny[i] + h[i])
        .fold(0.0_f32, f32::max);
    let canon_w = max_right + margin;
    let canon_h = max_bottom + margin;
    (nx, ny, canon_w, canon_h)
}

/// Transform a canonical-space position into the rendered top-left corner
/// for the target direction. Only the position is transformed — the box is
/// always drawn at its text-fitted size. `lh` is the node's layout footprint
/// along the rank axis (needed to flip correctly for bottom-up / right-left);
/// `canon_h` is the canonical canvas height (the full rank extent).
fn transform_pos(p: (f32, f32, f32), dir: Direction, canon_h: f32) -> (f32, f32) {
    let (x, y, lh) = p;
    match dir {
        Direction::TopDown => (x, y),
        Direction::BottomUp => (x, canon_h - (y + lh)),
        Direction::LeftRight => (y, x),
        Direction::RightLeft => (canon_h - (y + lh), x),
    }
}

/// Transform a canonical point (e.g. an edge port/waypoint) into the target
/// direction. `canon_h` is the canonical canvas height (the rank extent).
fn transform_point(p: (f32, f32), dir: Direction, canon_h: f32) -> (f32, f32) {
    let (x, y) = p;
    match dir {
        Direction::TopDown => (x, y),
        Direction::BottomUp => (x, canon_h - y),
        Direction::LeftRight => (y, x),
        Direction::RightLeft => (canon_h - y, x),
    }
}

/// Final canvas dimensions for a direction given the canonical size.
fn target_dims(canon_w: f32, canon_h: f32, dir: Direction) -> (f32, f32) {
    match dir {
        Direction::TopDown | Direction::BottomUp => (canon_w, canon_h),
        Direction::LeftRight | Direction::RightLeft => (canon_h, canon_w),
    }
}

// ============ Edge waypoints ============

/// Build the canonical (top-down) waypoints for one original edge along its
/// DAG path. The first point is the source-facing port of the low-rank end;
/// the last is the target-facing port of the high-rank end; intermediate
/// dummy centers route the edge through the gaps. If the edge was reversed
/// for cycle removal, the points are flipped so they still run `from` → `to`.
fn edge_waypoints(e: &OrigEdge, x: &[f32], y: &[f32], w: &[f32], h: &[f32]) -> Vec<(f32, f32)> {
    if e.path.is_empty() {
        return Vec::new();
    }
    if e.path.len() == 1 {
        // Self-loop: emit a small stub below the node (real routing in M7).
        let v = e.path[0];
        let cx = x[v] + w[v] / 2.0;
        let bottom = y[v] + h[v];
        return vec![(cx, bottom), (cx, bottom + 14.0)];
    }
    let lo = e.path[0];
    let hi = *e.path.last().unwrap();
    let mut pts = Vec::with_capacity(e.path.len());
    // Low-rank end leaves from its bottom-center.
    pts.push((x[lo] + w[lo] / 2.0, y[lo] + h[lo]));
    // Intermediate dummy centers.
    for &d in &e.path[1..e.path.len() - 1] {
        pts.push((x[d] + w[d] / 2.0, y[d] + h[d] / 2.0));
    }
    // High-rank end enters at its top-center.
    pts.push((x[hi] + w[hi] / 2.0, y[hi]));
    if e.reversed {
        pts.reverse();
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Shape;
    use crate::parser;
    use crate::resolve;

    fn lay(src: &str) -> (crate::ast::Diagram, Layout) {
        let raw = parser::parse_diagram(src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let l = layout(&d);
        (d, l)
    }

    fn node_rect<'a>(l: &'a Layout, id: &str) -> &'a NodeRect {
        l.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node `{id}` in layout"))
    }

    fn edge_path<'a>(l: &'a Layout, from: &str, to: &str) -> &'a EdgePath {
        l.edges
            .iter()
            .find(|e| e.from == from && e.to == to)
            .unwrap_or_else(|| panic!("no edge `{from}->{to}` in layout"))
    }

    fn sub_rect(l: &Layout, idx: usize) -> &SubgraphRect {
        l.subgraphs
            .iter()
            .find(|s| s.index == idx)
            .unwrap_or_else(|| panic!("no subgraph frame #{idx}"))
    }

    /// Do two axis-aligned rectangles overlap (with a tiny epsilon)?
    fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        ax < bx + bw - 1e-3
            && bx < ax + aw - 1e-3
            && ay < by + bh - 1e-3
            && by < ay + ah - 1e-3
    }

    /// `b` fully contains `a` (with a tiny epsilon).
    fn contains(outer: (f32, f32, f32, f32), inner: (f32, f32, f32, f32)) -> bool {
        let (ox, oy, ow, oh) = outer;
        let (ix, iy, iw, ih) = inner;
        ix >= ox - 1e-3
            && iy >= oy - 1e-3
            && ix + iw <= ox + ow + 1e-3
            && iy + ih <= oy + oh + 1e-3
    }

    /// Assert no two *real* node rectangles overlap.
    fn assert_no_overlaps(l: &Layout) {
        for i in 0..l.nodes.len() {
            for j in (i + 1)..l.nodes.len() {
                let a = &l.nodes[i];
                let b = &l.nodes[j];
                assert!(
                    !overlaps(
                        (a.x, a.y, a.w, a.h),
                        (b.x, b.y, b.w, b.h),
                    ),
                    "overlap between {} and {}",
                    a.id,
                    b.id
                );
            }
        }
    }

    fn assert_all_finite(l: &Layout) {
        for n in &l.nodes {
            assert!(n.x.is_finite() && n.y.is_finite() && n.w.is_finite() && n.h.is_finite());
            assert!(n.w > 0.0 && n.h > 0.0, "{} has zero size", n.id);
        }
        for e in &l.edges {
            for &(px, py) in &e.points {
                assert!(px.is_finite() && py.is_finite());
            }
        }
        for s in &l.subgraphs {
            assert!(s.w > 0.0 && s.h > 0.0, "subgraph {} has zero size", s.index);
        }
        assert!(l.width.is_finite() && l.height.is_finite());
        assert!(l.width > 0.0 && l.height > 0.0);
    }

    #[test]
    fn infra_example_layouts() {
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        assert_eq!(l.nodes.len(), 8);
        assert_eq!(l.edges.len(), 10);
        assert_eq!(l.subgraphs.len(), 1);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_subdirection_for_inspection() {
        let (d, l) = lay(include_str!("../examples/subdirection.mmd"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        for n in &nodes {
            println!("  {:<14} x={:7.1} y={:7.1} w={:5.1} h={:5.1}", n.id, n.x, n.y, n.w, n.h);
        }
        for s in &l.subgraphs {
            println!("  subgraph #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}", s.index, d.subgraphs[s.index].title, s.x, s.y, s.w, s.h);
        }
        for e in &l.edges {
            let pts: Vec<String> = e.points.iter().map(|(x, y)| format!("({:.1},{:.1})", x, y)).collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_infra_for_inspection() {
        let (d, l) = lay(include_str!("../examples/infra.mmd"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        for n in &nodes {
            println!("  {:<14} x={:7.1} y={:7.1} w={:5.1} h={:5.1}", n.id, n.x, n.y, n.w, n.h);
        }
        for s in &l.subgraphs {
            println!("  subgraph #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}", s.index, d.subgraphs[s.index].title, s.x, s.y, s.w, s.h);
        }
        for e in &l.edges {
            let pts: Vec<String> = e.points.iter().map(|(x, y)| format!("({:.0},{:.0})", x, y)).collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn every_node_has_a_rect() {
        let (d, l) = lay(include_str!("../examples/infra.mmd"));
        for n in &d.nodes {
            assert!(l.nodes.iter().any(|r| r.id == n.id), "missing rect for {}", n.id);
        }
    }

    #[test]
    fn empty_diagram_is_empty_layout() {
        let (_d, l) = lay("diagram top-down\n");
        assert!(l.nodes.is_empty());
        assert!(l.edges.is_empty());
        assert_eq!(l.width, 0.0);
        assert_eq!(l.height, 0.0);
    }

    #[test]
    fn isolated_nodes_share_a_row() {
        // Three disconnected nodes, top-down: all rank 0, laid in one row.
        let (_d, l) = lay("diagram top-down\na\nb\nc\n");
        assert_eq!(l.nodes.len(), 3);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // Same row ⇒ same top y (up to float equality, since one band).
        let y0 = l.nodes[0].y;
        assert!(l.nodes.iter().all(|n| (n.y - y0).abs() < 1e-3));
        // Distinct x.
        let xs: Vec<f32> = l.nodes.iter().map(|n| n.x).collect();
        assert_eq!(xs.len(), 3);
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "expected left-to-right order {xs:?}");
    }

    #[test]
    fn chain_aligns_vertically_top_down() {
        let (_d, l) = lay("diagram top-down\na-->b-->c-->d\n");
        // All on the same vertical line ⇒ equal x-centers.
        let cx = |id: &str| {
            let r = node_rect(&l, id);
            r.x + r.w / 2.0
        };
        for (a, b) in [("a", "b"), ("b", "c"), ("c", "d")] {
            assert!((cx(a) - cx(b)).abs() < 1e-2, "chain {a}->{b} not aligned");
        }
        // And strictly downward.
        assert!(node_rect(&l, "a").y < node_rect(&l, "b").y);
        assert!(node_rect(&l, "b").y < node_rect(&l, "c").y);
        assert!(node_rect(&l, "c").y < node_rect(&l, "d").y);
    }

    #[test]
    fn left_right_chain_is_horizontal() {
        let (_d, l) = lay("diagram left-right\na-->b-->c\n");
        assert!(node_rect(&l, "a").x < node_rect(&l, "b").x);
        assert!(node_rect(&l, "b").x < node_rect(&l, "c").x);
        // Same horizontal line ⇒ equal y-centers.
        let cy = |id: &str| {
            let r = node_rect(&l, id);
            r.y + r.h / 2.0
        };
        assert!((cy("a") - cy("b")).abs() < 1e-2);
        assert!((cy("b") - cy("c")).abs() < 1e-2);
    }

    #[test]
    fn horizontal_direction_keeps_boxes_wide_short() {
        // Labels are never rotated, so a box must always be wide enough to fit
        // its horizontal text — even in left-right, where the rank axis is
        // horizontal. Only the box's *position* is rotated, not its size.
        let (_d, l) = lay("diagram left-right\nauth --> users --> orders\n");
        for id in ["auth", "users", "orders"] {
            let r = node_rect(&l, id);
            // Wide-and-short, never tall-and-narrow.
            assert!(
                r.w >= r.h,
                "{id} should be wide-short in left-right (w={:.2} h={:.2})",
                r.w,
                r.h
            );
            // And wide enough to actually contain the label text (plus the
            // same horizontal padding the top-down path uses).
            let label = id; // bare id ⇒ label == id
            let m = text::measure(label, FONT_SIZE);
            assert!(
                r.w >= m.width + 2.0 * NODE_PAD_X - 1e-2,
                "{id} box too narrow for its label: w={:.2} need {:.2}",
                r.w,
                m.width + 2.0 * NODE_PAD_X
            );
        }
    }

    #[test]
    fn left_right_internal_edges_use_horizontal_ports() {
        // In left-right, an internal edge should leave the source's right side
        // and enter the target's left side (horizontal flow), not the
        // top/bottom.
        let (_d, l) = lay("diagram left-right\nalpha --> beta\n");
        let a = node_rect(&l, "alpha");
        let b = node_rect(&l, "beta");
        assert!(a.x < b.x, "alpha should be left of beta");
        let e = edge_path(&l, "alpha", "beta");
        let (p0, p1) = (e.points[0], *e.points.last().unwrap());
        // Start on alpha's right edge, at its vertical center.
        assert!((p0.0 - (a.x + a.w)).abs() < 1e-2, "start not on alpha's right: {p0:?} a={a:?}");
        assert!((p0.1 - (a.y + a.h / 2.0)).abs() < 1e-2);
        // End on beta's left edge, at its vertical center.
        assert!((p1.0 - b.x).abs() < 1e-2, "end not on beta's left: {p1:?} b={b:?}");
        assert!((p1.1 - (b.y + b.h / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn direction_swaps_dimensions_for_a_chain() {
        let (_d, l_td) = lay("diagram top-down\na-->b-->c-->d\n");
        let (_d, l_lr) = lay("diagram left-right\na-->b-->c-->d\n");
        // A 4-rank chain is tall top-down, wide left-right.
        assert!(l_td.height > l_td.width, "top-down chain should be tall");
        assert!(l_lr.width > l_lr.height, "left-right chain should be wide");
    }

    #[test]
    fn cycle_is_handled_without_overlap() {
        // A <-> B (a 2-cycle). One edge is reversed internally; both still draw.
        let (_d, l) = lay("diagram top-down\na-->b\nb-->a\n");
        assert_eq!(l.nodes.len(), 2);
        assert_eq!(l.edges.len(), 2);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // One edge goes top-to-bottom, the other bottom-to-top (reversed).
        let e1 = edge_path(&l, "a", "b");
        let e2 = edge_path(&l, "b", "a");
        assert!(!e1.points.is_empty() && !e2.points.is_empty());
    }

    #[test]
    fn long_edge_gets_intermediate_waypoint() {
        // A->C spans two ranks (B sits between); A->C should route via a dummy.
        let (_d, l) = lay("diagram top-down\na-->b\nc\nb-->c\na-->c\n");
        let ac = edge_path(&l, "a", "c");
        // path = [a, dummy@rank1, c] ⇒ 3 waypoints.
        assert!(
            ac.points.len() >= 3,
            "long edge a->c should have >=3 waypoints, got {}",
            ac.points.len()
        );
    }

    #[test]
    fn self_loop_does_not_panic() {
        let (_d, l) = lay("diagram top-down\na-->a\n");
        assert_eq!(l.nodes.len(), 1);
        assert_eq!(l.edges.len(), 1);
        assert_all_finite(&l);
        assert!(!l.edges[0].points.is_empty());
    }

    #[test]
    fn crossing_minimization_resolves_a_swap() {
        // Two sources {A,B}, two sinks {C,D}. A->D and B->C cross if the
        // layers are kept in input order; barycenter should flip one layer to
        // remove the crossing. Zero crossings requires the relative order of
        // each edge's own endpoints to agree: (order(a) < order(b)) ==
        // (order(d) < order(c)).
        let src = "diagram top-down\na\nc\nb\nd\na-->d\nb-->c\n";
        let (_d, l) = lay(src);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        let ax = node_rect(&l, "a").x + node_rect(&l, "a").w / 2.0;
        let bx = node_rect(&l, "b").x + node_rect(&l, "b").w / 2.0;
        let cx = node_rect(&l, "c").x + node_rect(&l, "c").w / 2.0;
        let dx = node_rect(&l, "d").x + node_rect(&l, "d").w / 2.0;
        let top_order = ax < bx; // order(a) < order(b)
        let bottom_order = dx < cx; // order(d) < order(c)
        assert_eq!(
            top_order, bottom_order,
            "crossing not removed: edges a->d, b->c still cross"
        );
    }

    #[test]
    fn diamond_lays_out_symmetric() {
        let (_d, l) = lay("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        let cx = |id: &str| node_rect(&l, id).x + node_rect(&l, id).w / 2.0;
        // The apex and sink sit between the two middle nodes.
        let mid_bc = (cx("b") + cx("c")) / 2.0;
        assert!((cx("a") - mid_bc).abs() < (cx("c") - cx("b")).abs() + 1e-2);
        assert!((cx("d") - mid_bc).abs() < (cx("c") - cx("b")).abs() + 1e-2);
        // b strictly left of c.
        assert!(cx("b") < cx("c"));
    }

    #[test]
    fn bottom_up_reverses_top_down() {
        let src = "diagram top-down\na-->b-->c\n";
        let (_d, l_td) = lay(src);
        let (_d, l_bu) = lay("diagram bottom-up\na-->b-->c\n");
        // In top-down, a is at the top (smallest y); in bottom-up, a is at the
        // bottom (largest y).
        let a_td = node_rect(&l_td, "a");
        let a_bu = node_rect(&l_bu, "a");
        assert!(a_td.y < a_bu.y, "a should be higher in top-down than bottom-up");
        let c_td = node_rect(&l_td, "c");
        let c_bu = node_rect(&l_bu, "c");
        assert!(c_bu.y < c_td.y, "c should be lower in bottom-up than top-down");
    }

    #[test]
    fn right_left_reverses_left_right() {
        let (_d, l_lr) = lay("diagram left-right\na-->b-->c\n");
        let (_d, l_rl) = lay("diagram right-left\na-->b-->c\n");
        // In left-right, a is leftmost (small x); in right-left, a is rightmost.
        let a_lr = node_rect(&l_lr, "a");
        let a_rl = node_rect(&l_rl, "a");
        assert!(a_lr.x < a_rl.x);
        let c_lr = node_rect(&l_lr, "c");
        let c_rl = node_rect(&l_rl, "c");
        assert!(c_rl.x < c_lr.x);
    }

    #[test]
    fn edge_endpoints_touch_node_bounds() {
        let (_d, l) = lay("diagram top-down\na-->b\n");
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let e = edge_path(&l, "a", "b");
        let (p0, p1) = (e.points[0], e.points[1]);
        // First point on a's bottom edge, at its horizontal center.
        assert!((p0.1 - (a.y + a.h)).abs() < 1e-2, "start not on a's bottom: {p0:?} a={a:?}");
        assert!((p0.0 - (a.x + a.w / 2.0)).abs() < 1e-2);
        // Last point on b's top edge, at its horizontal center.
        assert!((p1.1 - b.y).abs() < 1e-2, "end not on b's top: {p1:?} b={b:?}");
        assert!((p1.0 - (b.x + b.w / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn cylinder_is_taller_than_a_box_for_the_same_label() {
        // A cylinder needs room for its elliptical caps plus breathing space
        // for the centered label (see `cyl_height`), so it is taller than a
        // box with the same label — and tall enough that the label clears the
        // lid (the guarantee exercised in the render tests).
        let (_d, l) = lay(
            "diagram top-down\n\n\
             box \"Pg\"\n\
             cyl \"Pg\" : cylinder\n",
        );
        let box_h = node_rect(&l, "box").h;
        let cyl_h = node_rect(&l, "cyl").h;
        assert!(cyl_h > box_h, "cylinder ({cyl_h}) should be taller than box ({box_h})");
        // Concretely: box = text + 2·pad = 28; cylinder adds 2·CYL_PAD + 2·CYL_RY.
        assert_eq!(box_h, 28.0);
        assert_eq!(cyl_h, 28.0 + 2.0 * CYL_PAD + 2.0 * CYL_RY);
        // The label clears the lid's lowest point (em box above it).
        let r = node_rect(&l, "cyl");
        let cy = r.y + r.h / 2.0;
        let glyph_top = cy - FONT_SIZE / 2.0;
        let lid_bottom = r.y + 2.0 * CYL_RY;
        assert!(glyph_top >= lid_bottom + 3.0, "label would collide with lid");
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        let _ = Shape::Cylinder;
    }

    // ================= Compound layout (M4) =================

    #[test]
    fn subgraph_frame_contains_its_members() {
        let (d, l) = lay(
            "diagram top-down\n\
             subgraph \"Cluster\"\n\
             a\n\
             b\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 1);
        assert_eq!(l.nodes.len(), 2);
        let f = sub_rect(&l, 0);
        assert_eq!(d.subgraphs[0].title.as_deref(), Some("Cluster"));
        for id in ["a", "b"] {
            let n = node_rect(&l, id);
            assert!(
                contains((f.x, f.y, f.w, f.h), (n.x, n.y, n.w, n.h)),
                "node {id} not inside its frame (frame={f:?} node={n:?})"
            );
            // And with a real inset (not hugging the border).
            assert!(n.x > f.x + 1.0 && n.y > f.y + 1.0);
        }
        assert_all_finite(&l);
    }

    #[test]
    fn nested_subgraph_frame_inside_outer_frame() {
        let (d, l) = lay(
            "diagram top-down\n\
             subgraph \"Outer\"\n\
             subgraph \"Inner\"\n\
             a\n\
             end\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 2);
        let outer = sub_rect(&l, 0);
        let inner = sub_rect(&l, 1);
        assert_eq!(d.subgraphs[0].title.as_deref(), Some("Outer"));
        assert_eq!(d.subgraphs[1].title.as_deref(), Some("Inner"));
        assert!(
            contains((outer.x, outer.y, outer.w, outer.h), (inner.x, inner.y, inner.w, inner.h)),
            "inner frame not inside outer frame"
        );
        let a = node_rect(&l, "a");
        assert!(
            contains((inner.x, inner.y, inner.w, inner.h), (a.x, a.y, a.w, a.h)),
            "node a not inside inner frame"
        );
        assert_all_finite(&l);
    }

    #[test]
    fn subgraph_inherits_diagram_direction() {
        // A subgraph with no explicit direction inherits the diagram's
        // direction. With left-right, two isolated members share a column
        // (stacked vertically), not a row.
        let (_d, l) = lay(
            "diagram left-right\n\
             subgraph \"S\"\n\
             a\n\
             b\n\
             end\n",
        );
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        // left-right ⇒ ranks flow along x, so two isolated nodes are stacked
        // vertically ⇒ distinct y, same x-center.
        assert!((a.y - b.y).abs() > 1.0, "members should stack vertically under left-right");
        assert!(((a.x + a.w / 2.0) - (b.x + b.w / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn per_subgraph_direction_changes_internal_arrangement() {
        // Same internal graph (a chain) under two different subgraph
        // directions. Top-down ⇒ frame is tall; left-right ⇒ frame is wide.
        let src_td = "diagram top-down\n\
             subgraph top-down \"S\"\n\
             a --> b --> c\n\
             end\n";
        let src_lr = "diagram top-down\n\
             subgraph left-right \"S\"\n\
             a --> b --> c\n\
             end\n";
        let (_d, l_td) = lay(src_td);
        let (_d, l_lr) = lay(src_lr);
        let f_td = sub_rect(&l_td, 0);
        let f_lr = sub_rect(&l_lr, 0);

        // Internal chain geometry: top-down ⇒ a above b above c (distinct y);
        // left-right ⇒ a left of b left of c (distinct x).
        let (a_td, b_td, c_td) = (
            node_rect(&l_td, "a"),
            node_rect(&l_td, "b"),
            node_rect(&l_td, "c"),
        );
        assert!(a_td.y < b_td.y && b_td.y < c_td.y, "top-down subgraph should stack vertically");
        let (a_lr, b_lr, c_lr) = (
            node_rect(&l_lr, "a"),
            node_rect(&l_lr, "b"),
            node_rect(&l_lr, "c"),
        );
        assert!(a_lr.x < b_lr.x && b_lr.x < c_lr.x, "left-right subgraph should run horizontally");

        // The frame aspect ratio flips between the two.
        assert!(
            f_td.h > f_td.w,
            "top-down subgraph frame should be tall (w={:.1} h={:.1})",
            f_td.w,
            f_td.h
        );
        assert!(
            f_lr.w > f_lr.h,
            "left-right subgraph frame should be wide (w={:.1} h={:.1})",
            f_lr.w,
            f_lr.h
        );
    }

    #[test]
    fn top_level_nodes_and_subgraph_coexist_without_overlap() {
        let (_d, l) = lay(
            "diagram top-down\n\
             x\n\
             subgraph \"S\"\n\
             a\n\
             b\n\
             end\n",
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // The top-level node must not overlap the frame (they are siblings
        // at the top level, laid out with a separation gap).
        let x = node_rect(&l, "x");
        let f = sub_rect(&l, 0);
        assert!(
            !overlaps((x.x, x.y, x.w, x.h), (f.x, f.y, f.w, f.h)),
            "top-level node x overlaps subgraph frame"
        );
    }

    #[test]
    fn cross_boundary_edge_renders_between_actual_endpoints() {
        // lb (top-level) -> api (inside subgraph): a cross-boundary edge.
        let (_d, l) = lay(
            "diagram top-down\n\
             lb\n\
             subgraph \"S\"\n\
             api\n\
             end\n\
             lb --> api\n",
        );
        assert_eq!(l.edges.len(), 1);
        let e = edge_path(&l, "lb", "api");
        assert!(e.points.len() >= 2, "cross-boundary edge needs waypoints");
        assert_all_finite(&l);
        // First point sits on lb's boundary; last on api's boundary.
        let lb = node_rect(&l, "lb");
        let api = node_rect(&l, "api");
        let p0 = e.points[0];
        let p1 = *e.points.last().unwrap();
        let on_lb = (p0.0 - lb.x).abs() < 1e-2
            || (p0.0 - (lb.x + lb.w)).abs() < 1e-2
            || (p0.1 - lb.y).abs() < 1e-2
            || (p0.1 - (lb.y + lb.h)).abs() < 1e-2;
        assert!(on_lb, "cross-boundary edge start not on lb's boundary: {p0:?}");
        let on_api = (p1.0 - api.x).abs() < 1e-2
            || (p1.0 - (api.x + api.w)).abs() < 1e-2
            || (p1.1 - api.y).abs() < 1e-2
            || (p1.1 - (api.y + api.h)).abs() < 1e-2;
        assert!(on_api, "cross-boundary edge end not on api's boundary: {p1:?}");
    }

    #[test]
    fn internal_subgraph_edge_uses_internal_layout() {
        // An edge fully inside a subgraph is a direct internal edge: laid out
        // within the subgraph, so b sits strictly below a (top-down subgraph).
        let (_d, l) = lay(
            "diagram top-down\n\
             subgraph \"S\"\n\
             a --> b\n\
             end\n",
        );
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        assert!(a.y < b.y, "internal edge a->b should stack a above b");
        let e = edge_path(&l, "a", "b");
        assert!(e.points.len() >= 2);
        let f = sub_rect(&l, 0);
        // Both nodes stay inside the frame.
        assert!(contains((f.x, f.y, f.w, f.h), (a.x, a.y, a.w, a.h)));
        assert!(contains((f.x, f.y, f.w, f.h), (b.x, b.y, b.w, b.h)));
    }

    #[test]
    fn cross_boundary_edge_does_not_collapse_internal_direction() {
        // The headline guarantee: a subgraph with its own direction keeps
        // that internal arrangement even when edges cross its boundary.
        // Here the subgraph is left-right with a chain; external edges attach
        // to its members but the chain still runs horizontally.
        let (_d, l) = lay(
            "diagram top-down\n\
             src\n\
             sink\n\
             subgraph left-right \"S\"\n\
             a --> b --> c\n\
             end\n\
             src --> a\n\
             c --> sink\n",
        );
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        assert!(a.x < b.x && b.x < c.x, "left-right subgraph chain must stay horizontal");
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // Both cross-boundary edges render.
        assert!(edge_path(&l, "src", "a").points.len() >= 2);
        assert!(edge_path(&l, "c", "sink").points.len() >= 2);
    }

    #[test]
    fn every_descendant_node_present_with_nested_subgraphs() {
        let (_d, l) = lay(
            "diagram top-down\n\
             subgraph \"A\"\n\
             subgraph \"B\"\n\
             subgraph \"C\"\n\
             x --> y\n\
             end\n\
             end\n\
             end\n",
        );
        assert_eq!(l.nodes.len(), 2);
        assert_eq!(l.subgraphs.len(), 3);
        assert_all_finite(&l);
        // x -> y is a direct internal edge of the innermost subgraph.
        assert!(edge_path(&l, "x", "y").points.len() >= 2);
        // Frames nest: A ⊃ B ⊃ C.
        let a = sub_rect(&l, 0);
        let b = sub_rect(&l, 1);
        let c = sub_rect(&l, 2);
        assert!(contains((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h)));
        assert!(contains((b.x, b.y, b.w, b.h), (c.x, c.y, c.w, c.h)));
    }

    #[test]
    fn empty_subgraph_renders_a_frame() {
        let (d, l) = lay(
            "diagram top-down\n\
             subgraph \"Empty\"\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 1);
        let f = sub_rect(&l, 0);
        assert_eq!(d.subgraphs[0].title.as_deref(), Some("Empty"));
        assert!(f.w > 0.0 && f.h > 0.0, "empty subgraph should still have a visible frame");
    }

    // ================= M5 — cross-boundary frame routing =================

    /// Is `p` on the perimeter of axis-aligned rect `r` (with epsilon)?
    fn on_boundary(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
        let (x, y, w, h) = r;
        let (px, py) = p;
        let eps = 1e-2;
        let within_x = px >= x - eps && px <= x + w + eps;
        let within_y = py >= y - eps && py <= y + h + eps;
        let on_lr = (px - x).abs() < eps || (px - (x + w)).abs() < eps;
        let on_tb = (py - y).abs() < eps || (py - (y + h)).abs() < eps;
        (on_lr && within_y) || (on_tb && within_x)
    }

    /// Does segment `p0`→`p1` intersect the closed axis-aligned rect `r`?
    /// (Liang–Barsky line clipping.)
    fn segment_intersects_rect(
        p0: (f32, f32),
        p1: (f32, f32),
        r: (f32, f32, f32, f32),
    ) -> bool {
        let (x0, y0) = p0;
        let (x1, y1) = p1;
        let (rx, ry, rw, rh) = r;
        let dx = x1 - x0;
        let dy = y1 - y0;
        let mut t0 = 0.0_f32;
        let mut t1 = 1.0_f32;
        for (p, q) in [
            (-dx, x0 - rx),
            (dx, rx + rw - x0),
            (-dy, y0 - ry),
            (dy, ry + rh - y0),
        ] {
            if p.abs() < 1e-9 {
                if q < -1e-9 {
                    return false;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    if t > t1 {
                        return false;
                    }
                    if t > t0 {
                        t0 = t;
                    }
                } else {
                    if t < t0 {
                        return false;
                    }
                    if t < t1 {
                        t1 = t;
                    }
                }
            }
        }
        t0 < t1 - 1e-6
    }

    /// A node/subgraph rect as a tuple, for the geometry helpers above.
    fn rect_of(n: &NodeRect) -> (f32, f32, f32, f32) {
        (n.x, n.y, n.w, n.h)
    }
    fn sub_rect_of(s: &SubgraphRect) -> (f32, f32, f32, f32) {
        (s.x, s.y, s.w, s.h)
    }

    #[test]
    fn cross_boundary_edge_routes_through_frame_connection_point() {
        // lb (top-level) -> api (inside S): a cross-boundary edge. It must
        // route through a connection point on S's frame rather than piercing
        // it. With lb above S (top-down), the connection point is on S's top
        // edge, at S's center-x (the representative's port).
        let (_d, l) = lay(r#"diagram top-down
lb
subgraph "S"
api
end
lb --> api
"#);
        let e = edge_path(&l, "lb", "api");
        assert_eq!(
            e.points.len(),
            3,
            "lb->api should be [lb port, S port, api port], got {:?}",
            e.points
        );
        let lb = node_rect(&l, "lb");
        let api = node_rect(&l, "api");
        let s = sub_rect(&l, 0);
        // First point: lb's bottom-center (exit side, facing S below).
        assert!((e.points[0].1 - (lb.y + lb.h)).abs() < 1e-2, "start not on lb's bottom");
        assert!((e.points[0].0 - (lb.x + lb.w / 2.0)).abs() < 1e-2);
        // Middle point: a connection point on S's top edge at S's center-x.
        assert!(
            on_boundary(sub_rect_of(s), e.points[1]),
            "connection point {:?} not on S's frame {:?}",
            e.points[1],
            s
        );
        assert!((e.points[1].1 - s.y).abs() < 1e-2, "connection point not on S's top edge");
        assert!((e.points[1].0 - (s.x + s.w / 2.0)).abs() < 1e-2, "connection point not at S's center-x");
        // Last point: api's top-center (entry side).
        assert!((e.points[2].1 - api.y).abs() < 1e-2, "end not on api's top");
        assert!((e.points[2].0 - (api.x + api.w / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn cross_boundary_edge_connects_two_frames() {
        // a (in A) -> b (in B): both endpoints are inside frames. The path
        // runs a port -> A port -> B port -> b port, with a connection point
        // on each frame.
        let (_d, l) = lay(r#"diagram top-down
subgraph "A"
a
end
subgraph "B"
b
end
a --> b
"#);
        let e = edge_path(&l, "a", "b");
        assert_eq!(
            e.points.len(),
            4,
            "a->b should be [a port, A port, B port, b port], got {:?}",
            e.points
        );
        let a_frame = sub_rect(&l, 0);
        let b_frame = sub_rect(&l, 1);
        // A's connection point: on A's bottom edge (A is above B), at A's center-x.
        assert!((e.points[1].1 - (a_frame.y + a_frame.h)).abs() < 1e-2, "A port not on A's bottom");
        assert!((e.points[1].0 - (a_frame.x + a_frame.w / 2.0)).abs() < 1e-2);
        // B's connection point: on B's top edge, at B's center-x.
        assert!((e.points[2].1 - b_frame.y).abs() < 1e-2, "B port not on B's top");
        assert!((e.points[2].0 - (b_frame.x + b_frame.w / 2.0)).abs() < 1e-2);
        assert!(on_boundary(sub_rect_of(a_frame), e.points[1]));
        assert!(on_boundary(sub_rect_of(b_frame), e.points[2]));
    }

    #[test]
    fn cross_boundary_frame_points_preserve_subgraph_direction() {
        // The headline M5 guarantee: a subgraph's own direction is kept even
        // when edges cross its frame, AND those edges route through frame
        // connection points rather than piercing the frame.
        let (_d, l) = lay(r#"diagram top-down
src
sink
subgraph left-right "S"
a --> b --> c
end
src --> a
c --> sink
"#);
        // Per-subgraph direction preserved: the chain stays horizontal.
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        assert!(a.x < b.x && b.x < c.x, "left-right subgraph chain must stay horizontal");
        // Both cross-boundary edges route through S frame connection points.
        let s = sub_rect(&l, 0);
        let s_rect = sub_rect_of(s);
        assert!(
            edge_path(&l, "src", "a").points.iter().any(|&p| on_boundary(s_rect, p)),
            "src->a should have a connection point on S's frame"
        );
        assert!(
            edge_path(&l, "c", "sink").points.iter().any(|&p| on_boundary(s_rect, p)),
            "c->sink should have a connection point on S's frame"
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }

    #[test]
    fn cross_boundary_stub_does_not_cross_sibling() {
        // The within-frame stub connects the endpoint node to its rep frame at
        // the *node's* cross-coordinate (not the frame center), so it runs
        // straight out along the node's own axis. With two members in a frame
        // and an edge from the right member to a node below-left, that straight
        // stub stays on the right member's side and must not cut across the
        // left member. This is the Mermaid/dagre failure mode this tool exists
        // to escape.
        //
        // Since M7 the stub is a right-angle Z (it jogs in the frame's bottom
        // padding, below the nodes), so the whole stub — every segment from
        // api2's port up to the K8s frame's connection point — is checked,
        // not just the first segment. The frame connection point itself is
        // found by value (it is no longer at a fixed index after the Z).
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let api1 = node_rect(&l, "api1");
        let api2 = node_rect(&l, "api2");
        let k8s = sub_rect(&l, 0);
        let e = edge_path(&l, "api2", "db");
        assert!(e.points.len() >= 3, "api2->db should route through K8s's frame");
        // The K8s frame connection point: on K8s's bottom edge, at api2's
        // center-x (the rep port lines up with the node, not the frame center).
        let api2_cx = api2.x + api2.w / 2.0;
        let k8s_bot_y = k8s.y + k8s.h;
        let cp_idx = e
            .points
            .iter()
            .position(|&p| {
                (p.0 - api2_cx).abs() < 1e-2 && (p.1 - k8s_bot_y).abs() < 1e-2
            })
            .expect("no K8s bottom connection point at api2's x on api2->db");
        assert!(
            cp_idx >= 1,
            "connection point should not be the first waypoint"
        );
        // Every segment of the stub (api2 port -> ... -> K8s connection point)
        // must miss api1.
        for seg in e.points[..cp_idx].windows(2) {
            assert!(
                !segment_intersects_rect(seg[0], seg[1], rect_of(api1)),
                "api2->db stub segment {:?}->{:?} crosses api1 {:?}",
                seg[0],
                seg[1],
                api1
            );
        }
        // The stub starts on api2's own bottom port (the exit side toward db).
        assert!((e.points[0].1 - (api2.y + api2.h)).abs() < 1e-2);
    }

    #[test]
    fn cross_boundary_edge_routes_around_frame_when_target_is_deep() {
        // Orders (in the left-right Services subgraph) -> Database (the
        // *bottom* of the top-down Storage subgraph, with Cache above it).
        // The straight within-frame stub would run straight through Cache and
        // then exactly overlap the Cache->Database edge (both at Storage's
        // center-x). Instead the edge routes around the Storage frame and
        // enters Database from the side, so it clears Cache and stays
        // distinct from the Cache->Database edge.
        let (_d, l) = lay(include_str!("../examples/subdirection.mmd"));
        let cache = node_rect(&l, "cache");
        let db = node_rect(&l, "db");
        let e = edge_path(&l, "orders", "db");

        // No segment of orders->db crosses the Cache node.
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], rect_of(cache)),
                "orders->db segment {:?}->{:?} crosses Cache {:?}",
                w[0],
                w[1],
                cache
            );
        }
        // orders->db enters Database from a side perpendicular to the flow
        // axis: its final segment is horizontal (the LCA is top-down) and its
        // last waypoint sits on Database's right edge at Database's center-y
        // — not on Database's top, which is where the overlapping stub would
        // have arrived.
        let (lp0, lp1) = (e.points[e.points.len() - 2], *e.points.last().unwrap());
        assert!(
            (lp0.1 - lp1.1).abs() < 1e-2,
            "orders->db last segment should be horizontal (side entry), got {:?}->{:?}",
            lp0,
            lp1
        );
        assert!(
            (lp1.0 - (db.x + db.w)).abs() < 1e-2,
            "orders->db should enter Database's right edge (x={:.2}), got x={:.2}",
            db.x + db.w,
            lp1.0
        );
        assert!(
            (lp1.1 - (db.y + db.h / 2.0)).abs() < 1e-2,
            "orders->db should enter at Database's center-y ({:.2}), got y={:.2}",
            db.y + db.h / 2.0,
            lp1.1
        );
        // The Cache->Database edge is vertical along Storage's center-x; the
        // two edges no longer coincide (orders->db arrives off that x).
        let cache_db = edge_path(&l, "cache", "db");
        assert!(
            (cache_db.points[0].0 - cache_db.points[cache_db.points.len() - 1].0).abs() < 1e-2,
            "cache->db should be vertical"
        );
        assert!(
            (lp1.0 - cache_db.points[0].0).abs() > 1e-2,
            "orders->db and cache->db arrive at the same x (would overlap)"
        );
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&l);
    }

    #[test]
    fn cross_boundary_edge_routes_around_frame_when_source_is_above_frame() {
        // src (top-level, centered above a top-down subgraph) -> b (the
        // *bottom* of the subgraph, with a above it). The straight within-
        // frame stub would run straight through a. The around-route drops
        // into the gap above the frame, runs down the frame's side, and
        // enters b from the side — the case where the source node is NOT
        // already clear of the frame on one side (it sits above the frame
        // within the frame's cross-span), so the route takes a gap jog first.
        let (_d, l) = lay(r#"diagram top-down
src
subgraph top-down "S"
a --> b
end
src --> b
"#);
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let e = edge_path(&l, "src", "b");
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], rect_of(a)),
                "src->b segment {:?}->{:?} crosses a {:?}",
                w[0],
                w[1],
                a
            );
        }
        // Enters b from a side (last segment horizontal in a top-down
        // diagram), on b's left or right edge at b's center-y.
        let (lp0, lp1) = (e.points[e.points.len() - 2], *e.points.last().unwrap());
        assert!(
            (lp0.1 - lp1.1).abs() < 1e-2,
            "src->b last segment should be horizontal (side entry), got {:?}->{:?}",
            lp0,
            lp1
        );
        assert!(
            (lp1.0 - (b.x + b.w)).abs() < 1e-2 || (lp1.0 - b.x).abs() < 1e-2,
            "src->b should enter b's left or right edge, got x={:.2}",
            lp1.0
        );
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&l);
    }

    #[test]
    fn cross_boundary_edge_crosses_each_nested_frame() {
        // ext (top-level) -> inner (inside Inner inside Outer): a cross-
        // boundary edge crossing two nested frames. It must record a
        // connection point on each frame it passes through.
        let (_d, l) = lay(r#"diagram top-down
ext
subgraph "Outer"
subgraph "Inner"
inner
end
end
ext --> inner
"#);
        let e = edge_path(&l, "ext", "inner");
        assert!(
            e.points.len() >= 4,
            "nested cross-boundary edge should cross both frames, got {:?}",
            e.points
        );
        let outer = sub_rect_of(sub_rect(&l, 0));
        let inner = sub_rect_of(sub_rect(&l, 1));
        assert!(
            e.points.iter().any(|&p| on_boundary(outer, p)),
            "no waypoint on Outer's frame: {:?}",
            e.points
        );
        assert!(
            e.points.iter().any(|&p| on_boundary(inner, p)),
            "no waypoint on Inner's frame: {:?}",
            e.points
        );
        assert_all_finite(&l);
    }

    #[test]
    fn cross_boundary_subgraph_grows_around_immediate_child() {
        // A subgraph that a cross-boundary edge reaches through (to an
        // *immediate* child) grows by [`CROSS_FRAME_PAD`] on top and bottom
        // so the edge's within-frame stub has room to jog clear of the title
        // text and the node's arrowhead. A subgraph with no such edge keeps
        // the default frame geometry. Both diagrams here hold the *same*
        // single-node content inside "S", so the only difference is the extra
        // padding.
        let with = lay(r#"diagram top-down
src
subgraph "S"
a
end
src --> a
"#);
        let without = lay(r#"diagram top-down
subgraph "S"
a
end
"#);
        let f_with = sub_rect(&with.1, 0);
        let f_without = sub_rect(&without.1, 0);
        // The cross-boundary subgraph is exactly 2*CROSS_FRAME_PAD taller
        // (one font-height on top, one on bottom).
        assert_eq!(
            f_with.h - f_without.h,
            2.0 * CROSS_FRAME_PAD,
            "cross-boundary frame should grow by 2*CROSS_FRAME_PAD: with.h={} without.h={}",
            f_with.h,
            f_without.h
        );
        // The top inset grew by CROSS_FRAME_PAD (the node sits that much
        // lower *relative to its frame*); the bottom padding grew by the
        // same. Compare the relative inset, not absolute y — the two frames
        // occupy different ranks, so their absolute y differs by far more.
        let a_with = node_rect(&with.1, "a");
        let a_without = node_rect(&without.1, "a");
        assert_eq!(
            a_with.y - f_with.y,
            FRAME_TITLE_H + CROSS_FRAME_PAD,
            "grown frame's top inset should be FRAME_TITLE_H + CROSS_FRAME_PAD"
        );
        assert_eq!(
            a_without.y - f_without.y,
            FRAME_TITLE_H,
            "untouched subgraph keeps the default top inset"
        );
        assert_eq!(
            (f_with.y + f_with.h) - (a_with.y + a_with.h),
            FRAME_PAD_Y + CROSS_FRAME_PAD,
            "grown frame's bottom padding should be FRAME_PAD_Y + CROSS_FRAME_PAD"
        );
    }

    #[test]
    fn cross_boundary_stub_jog_clears_title_region_and_arrowhead() {
        // The within-frame horizontal jog of a titled-top cross-boundary stub
        // must land in the clear padding *below* the frame's reserved title
        // space (so it does not touch the title text) and far enough *above*
        // the node that the arrowhead's body fits below it. Here src -> a
        // enters the titled frame from the top; a is the left end of a
        // left-right chain (a --> b) and sits under the (wide) title text, so
        // the stub detours past the title and jogs across to the node below it.
        // (A node already clear of the title would route straight, with no jog
        // to check.)
        let (_d, l) = lay(r#"diagram top-down
src
subgraph left-right "Services"
a --> b
end
src --> a
"#);
        let a = node_rect(&l, "a");
        let s = sub_rect(&l, 0);
        let e = edge_path(&l, "src", "a");
        // Find the within-frame horizontal jog (a same-y pair strictly between
        // the frame top and the node top).
        let mut jog_y: Option<f32> = None;
        for w in e.points.windows(2) {
            if (w[0].1 - w[1].1).abs() < 1e-2 && (w[0].0 - w[1].0).abs() > 1e-2
                && w[0].1 > s.y && w[0].1 < a.y {
                    jog_y = Some(w[0].1);
                    break;
                }
        }
        let jog_y = jog_y.expect("no within-frame horizontal jog on src->a");
        // The jog is exactly STUB_JOG_CLEARANCE above the node.
        assert!((jog_y - (a.y - STUB_JOG_CLEARANCE)).abs() < 1e-2);
        // STUB_JOG_CLEARANCE exceeds the arrowhead's back-extent (the render
        // marker is 10 px tall), so the jog clears the arrowhead body.
        const {
            assert!(
                STUB_JOG_CLEARANCE > 10.0,
                "STUB_JOG_CLEARANCE must exceed the arrowhead height so the jog clears it"
            );
        }
        // The jog lands at or below the frame's reserved title space
        // (frame.y + FRAME_TITLE_H), i.e. clear of the title text, which the
        // renderer draws within that reserved band. With the grown frame this
        // is the bottom of the band; the title glyphs occupy only its top.
        assert!(
            jog_y >= s.y + FRAME_TITLE_H - 1e-2,
            "jog {jog_y:.2} inside the title-inset band (should be at or below {:.2})",
            s.y + FRAME_TITLE_H
        );
        assert!(jog_y < a.y, "jog {jog_y:.2} not above the node top {:.2}", a.y);
    }

    // ================= M5.5 — centered block alignment =================

    /// Horizontal center of a node rect.
    fn cx(n: &NodeRect) -> f32 {
        n.x + n.w / 2.0
    }

    #[test]
    fn single_parent_rank_centers_block_under_parent() {
        // One parent fanning into three children of differing widths. The
        // children keep their left-to-right order and gaps, but the *block*
        // (span left..right) centers on the parent — not left-packed under
        // it.
        let (_d, l) = lay("diagram top-down\np\na\nbb\nccc\np-->a\np-->bb\np-->ccc\n");
        let p = node_rect(&l, "p");
        let a = node_rect(&l, "a");
        let bb = node_rect(&l, "bb");
        let ccc = node_rect(&l, "ccc");
        // Order preserved left to right.
        assert!(cx(a) < cx(bb) && cx(bb) < cx(ccc));
        // Block span center == parent center.
        let min_left = a.x.min(bb.x).min(ccc.x);
        let max_right = (a.x + a.w).max(bb.x + bb.w).max(ccc.x + ccc.w);
        let block_center = (min_left + max_right) / 2.0;
        assert!(
            (block_center - cx(p)).abs() < 1e-2,
            "child block center {block_center:.2} != parent center {:.2}",
            cx(p)
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }

    #[test]
    fn merge_node_centers_on_parents_midpoint() {
        // Two parents fanning into one child: the child centers on the
        // midpoint of its parents (a symmetric merge).
        let (_d, l) = lay("diagram top-down\na\nb\nc\na-->c\nb-->c\n");
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        assert!(cx(a) < cx(b), "parents keep their order");
        let parents_mid = (cx(a) + cx(b)) / 2.0;
        assert!(
            (cx(c) - parents_mid).abs() < 1e-2,
            "merge child center {:.2} != parents midpoint {:.2}",
            cx(c),
            parents_mid
        );
    }

    #[test]
    fn diamond_is_centered_on_one_axis() {
        // a -> {b,c} -> d. Apex a, sink d, and the b/c block all share one
        // center: the diagram is left-right symmetric.
        let (_d, l) = lay("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        let d = node_rect(&l, "d");
        assert!(cx(b) < cx(c), "b left of c");
        let bc_mid = (cx(b) + cx(c)) / 2.0;
        assert!((cx(a) - bc_mid).abs() < 1e-2, "apex not centered over b/c");
        assert!((cx(d) - bc_mid).abs() < 1e-2, "sink not centered over b/c");
        assert!((cx(a) - cx(d)).abs() < 1e-2, "apex and sink not aligned");
    }

    #[test]
    fn infra_web_lb_cluster_share_a_center() {
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let web = node_rect(&l, "web");
        let lb = node_rect(&l, "lb");
        let k8s = sub_rect(&l, 0);
        let web_c = cx(web);
        assert!((cx(lb) - web_c).abs() < 1e-2, "Load Balancer not aligned with Web Server");
        let k8s_c = k8s.x + k8s.w / 2.0;
        assert!((k8s_c - web_c).abs() < 1e-2, "K8s cluster frame not aligned with Web/LB");
    }

    #[test]
    fn infra_data_tier_centers_under_cluster() {
        // Postgres + Redis + Message Queue center as a block under the K8s
        // cluster (and thus under Web/LB).
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let db = node_rect(&l, "db");
        let cache = node_rect(&l, "cache");
        let queue = node_rect(&l, "queue");
        let k8s = sub_rect(&l, 0);
        let min_left = db.x.min(cache.x).min(queue.x);
        let max_right = (db.x + db.w).max(cache.x + cache.w).max(queue.x + queue.w);
        let block_center = (min_left + max_right) / 2.0;
        let k8s_c = k8s.x + k8s.w / 2.0;
        assert!(
            (block_center - k8s_c).abs() < 1e-2,
            "data tier block center {block_center:.2} != K8s center {k8s_c:.2}"
        );
        // And the three keep their left-to-right order.
        assert!(cx(db) < cx(cache) && cx(cache) < cx(queue));
    }

    #[test]
    fn infra_replica_centers_under_postgres() {
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let db = node_rect(&l, "db");
        let replica = node_rect(&l, "replica");
        assert!((cx(replica) - cx(db)).abs() < 1e-2, "Replica not aligned under Postgres");
    }

    // ================= M7 — orthogonal edge routing =================

    /// Every consecutive pair of waypoints shares an x or a y (axis-aligned).
    fn is_orthogonal(points: &[(f32, f32)]) -> bool {
        if points.len() < 2 {
            return true;
        }
        for w in points.windows(2) {
            let (ax, ay) = w[0];
            let (bx, by) = w[1];
            if (ax - bx).abs() >= 1e-2 && (ay - by).abs() >= 1e-2 {
                return false;
            }
        }
        true
    }

    #[test]
    fn all_edges_are_orthogonal() {
        // Every edge — direct, long, fork, merge, cross-boundary, cycled —
        // must render as an axis-aligned polyline after M7.
        let cases: &[(&str, &str)] = &[
            ("diagram top-down\na-->b-->c-->d\n", "chain"),
            ("diagram left-right\na-->b-->c\n", "left-right chain"),
            ("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n", "diamond"),
            ("diagram top-down\na-->b\nb-->a\n", "cycle"),
            (include_str!("../examples/infra.mmd"), "infra"),
            (include_str!("../examples/subdirection.mmd"), "subdirection"),
            (
                "diagram top-down\n\
                 src\n\
                 sink\n\
                 subgraph left-right \"S\"\n\
                 a --> b --> c\n\
                 end\n\
                 src --> a\n\
                 c --> sink\n",
                "cross-boundary into left-right subgraph",
            ),
        ];
        for (src, name) in cases {
            let (_d, l) = lay(src);
            assert_all_finite(&l);
            for e in &l.edges {
                assert!(
                    is_orthogonal(&e.points),
                    "edge {}->{} in `{name}` is not orthogonal: {:?}",
                    e.from,
                    e.to,
                    e.points
                );
            }
        }
    }

    #[test]
    fn fork_edge_gets_right_angle_bends() {
        // A fork a -> {b, c} with b and c on either side of a's center: the
        // edge to the off-center target is no longer a single diagonal but a
        // right-angle Z (down, across, down), so it has more than two
        // waypoints and is axis-aligned.
        let (_d, l) = lay("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        for (from, to) in [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")] {
            let e = edge_path(&l, from, to);
            assert!(e.points.len() > 2, "{from}->{to} should have bends: {:?}", e.points);
            assert!(is_orthogonal(&e.points), "{from}->{to} not orthogonal: {:?}", e.points);
        }
    }

    #[test]
    fn orthogonal_edge_endpoints_touch_node_bounds() {
        // Regression: orthogonalization must not move the first/last waypoint
        // off the source/target node boundary.
        let (_d, l) = lay("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        let on_boundary = |n: &NodeRect, p: (f32, f32)| {
            let eps = 1e-2;
            (p.0 - n.x).abs() < eps
                || (p.0 - (n.x + n.w)).abs() < eps
                || (p.1 - n.y).abs() < eps
                || (p.1 - (n.y + n.h)).abs() < eps
        };
        for (from, to) in [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")] {
            let e = edge_path(&l, from, to);
            let src = node_rect(&l, from);
            let dst = node_rect(&l, to);
            assert!(
                on_boundary(src, e.points[0]),
                "{from}->{to} start not on {from}: {:?}",
                e.points[0]
            );
            assert!(
                on_boundary(dst, *e.points.last().unwrap()),
                "{from}->{to} end not on {to}: {:?}",
                e.points.last().unwrap()
            );
        }
    }

    #[test]
    fn orthogonal_last_segment_is_cardinal() {
        // The final segment of every edge is axis-aligned, so the arrowhead's
        // `orient="auto"` orients it along a clean cardinal direction (the
        // arrow points straight at the target, not diagonally).
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        for e in &l.edges {
            assert!(e.points.len() >= 2);
            let n = e.points.len();
            let (p0, p1) = (e.points[n - 2], e.points[n - 1]);
            assert!(
                (p0.0 - p1.0).abs() < 1e-2 || (p0.1 - p1.1).abs() < 1e-2,
                "{}->{} last segment not axis-aligned: {:?}->{:?}",
                e.from,
                e.to,
                p0,
                p1
            );
        }
    }

    #[test]
    fn titled_frame_top_stub_jogs_near_node() {
        // lb -> api1 enters the titled K8s frame from the top. A naive
        // midpoint jog would land on the "Kubernetes Cluster" title; instead
        // the stub jogs [`STUB_JOG_CLEARANCE`] above the node, in the clear
        // padding below the title. The jog is closer to the node than to the
        // frame top (proving the near-node bias, not the midpoint default).
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let api1 = node_rect(&l, "api1");
        let k8s = sub_rect(&l, 0);
        let e = edge_path(&l, "lb", "api1");
        // The within-frame horizontal jog: a same-y pair strictly between the
        // frame top and the node top.
        let mut jog_y: Option<f32> = None;
        for w in e.points.windows(2) {
            if (w[0].1 - w[1].1).abs() < 1e-2 && (w[0].0 - w[1].0).abs() > 1e-2
                && w[0].1 > k8s.y && w[0].1 < api1.y {
                    jog_y = Some(w[0].1);
                    break;
                }
        }
        let jog_y = jog_y.expect("no within-frame horizontal jog on lb->api1");
        assert!(
            (jog_y - (api1.y - STUB_JOG_CLEARANCE)).abs() < 1e-2,
            "jog at {jog_y:.2}, expected api1.y - STUB_JOG_CLEARANCE = {:.2}",
            api1.y - STUB_JOG_CLEARANCE
        );
        let midpoint = (k8s.y + api1.y) / 2.0;
        assert!(jog_y > midpoint, "jog {jog_y:.2} not below midpoint {midpoint:.2}");
        assert!(jog_y < api1.y, "jog {jog_y:.2} not above api1 top {:.2}", api1.y);
    }

    #[test]
    fn cross_boundary_edges_enter_subgraph_aligned_with_target_node() {
        // Two cross-boundary edges into the same subgraph enter it at their
        // respective target node's cross-coordinate on the frame — not at the
        // frame's midpoint — so they stay distinct instead of converging on
        // one point (which read as both sources reaching both targets). The
        // frame's title is narrow here, so neither node sits under it and both
        // stubs run straight down to the node.
        let (_d, l) = lay(r#"diagram top-down
src1
src2
subgraph "S"
a
b
end
src1 --> a
src2 --> b
"#);
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let s = sub_rect(&l, 0);
        let s_top_y = s.y;
        let s_center = s.x + s.w / 2.0;
        // Each edge's connection point on S's top edge.
        let cp_x = |from: &str, to: &str| -> f32 {
            let e = edge_path(&l, from, to);
            e.points
                .iter()
                .find(|p| (p.1 - s_top_y).abs() < 1e-2)
                .unwrap_or_else(|| panic!("no connection point on S's top edge for {from}->{to}: {:?}", e.points))
                .0
        };
        let cp1 = cp_x("src1", "a");
        let cp2 = cp_x("src2", "b");
        assert!((cp1 - cx(a)).abs() < 1e-2, "src1->a enters S at x {cp1:.2}, not a's center {:.2}", cx(a));
        assert!((cp2 - cx(b)).abs() < 1e-2, "src2->b enters S at x {cp2:.2}, not b's center {:.2}", cx(b));
        assert!((cp1 - s_center).abs() > 1e-2, "src1->a merges at S's center-x");
        assert!((cp2 - s_center).abs() > 1e-2, "src2->b merges at S's center-x");
        assert!((cp1 - cp2).abs() > 1e-2, "the two edges share a connection point (merge)");
        assert!(is_orthogonal(&edge_path(&l, "src1", "a").points));
        assert!(is_orthogonal(&edge_path(&l, "src2", "b").points));
        assert_all_finite(&l);
    }

    #[test]
    fn cross_boundary_edge_between_aligned_nodes_is_straight() {
        // A cross-boundary edge whose two endpoints share a cross-coordinate
        // (here two single nodes in side-by-side grown subgraphs under a
        // left-right LCA) runs as a single straight line along that coordinate
        // — it does not jog up to a subgraph's frame-center coordinate and
        // back down. Before this change the rep port sat at each frame's
        // center, so the off-center node (pushed down by the grown frame's
        // extra top inset) forced exactly such a jog.
        let (_d, l) = lay(r#"diagram top-down
src
subgraph left-right "Outer"
    subgraph "A"
    a
    end
    subgraph "B"
    b
    end
end
src --> a
a --> b
"#);
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let fa = sub_rect(&l, 1); // "A"
        let fb = sub_rect(&l, 2); // "B"
        // The two nodes share a center-y (side-by-side grown frames, same rank).
        let ay = a.y + a.h / 2.0;
        let by = b.y + b.h / 2.0;
        assert!((ay - by).abs() < 1e-2, "a and b not aligned: {ay:.2} vs {by:.2}");
        // ... and that y is off each frame's center (the frames are grown), so a
        // frame-center port really would have jogged.
        assert!((ay - (fa.y + fa.h / 2.0)).abs() > 1e-2, "a sits at A's frame center");
        assert!((ay - (fb.y + fb.h / 2.0)).abs() > 1e-2, "b sits at B's frame center");
        // The edge a->b is a single straight horizontal line at that y.
        let e = edge_path(&l, "a", "b");
        assert!(e.points.len() >= 2);
        for p in &e.points {
            assert!((p.1 - ay).abs() < 1e-2, "a->b point {p:?} not on the node center-y {ay:.2}");
        }
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&l);
    }
}
