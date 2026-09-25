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
/// Horizontal padding inside a node box, each side: 1.5 font heights of
/// clearance beyond the label.
const NODE_PAD_X: f32 = 1.5 * FONT_SIZE;
/// Vertical padding inside a node box, each side. One font height per side
/// around the label's em box makes a node `3 * FONT_SIZE` tall — three font
/// heights total.
const NODE_PAD_Y: f32 = FONT_SIZE;
/// Minimum gap between two items in the same layer (edge to edge).
const NODE_SEP: f32 = 45.0;
/// Minimum gap between two consecutive layers (edge to edge).
const RANK_GAP: f32 = 67.5;
/// Page margin around the whole drawing (top level only).
const MARGIN: f32 = 20.0;
/// Smallest node size, so even tiny labels get a visible box. Match the
/// per-node sizing: three font heights, wide and tall.
const MIN_NODE_W: f32 = 3.0 * FONT_SIZE;
const MIN_NODE_H: f32 = 3.0 * FONT_SIZE;
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
const FRAME_PAD_X: f32 = 1.5 * FONT_SIZE;
const FRAME_PAD_Y: f32 = 8.0;
/// Height reserved at the top of a frame for the title, when present (one
/// title line; multi-line titles grow the inset — see [`frame_insets`]).
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
/// edges keeps the default frame geometry. Keep this in sync with the
/// renderer's subgraph title font size (`FRAME_TITLE_SIZE` in
/// `render/svg.rs`, 12 px): it is sized to one title-height of room. Its
/// horizontal mirror — growing a title-sized frame's *width* so a title
/// detour has an entry past the title — lives in [`layout_level`] (M10).
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
/// Base clearance between a title-detour entry and a subgraph title's edge
/// (see [`title_detour_clear_x`]). Applied to the title's **left** edge —
/// which is anchored at [`FRAME_TITLE_X`] by the renderer and so cannot be
/// widened by a viewer's font fallback — and, plus the fallback margins
/// ([`TITLE_FALLBACK_PAD`] / [`TITLE_FALLBACK_PER_W`]), to the title's
/// **right** edge. The gap clears the 1.5 px frame stroke and a little
/// breathing room.
const TITLE_CLEAR_GAP: f32 = 6.0;
/// Fixed clearance added past a title's *right* edge beyond
/// [`TITLE_CLEAR_GAP`] (see [`title_clear_gap`]). Text is measured with the
/// embedded DejaVu Sans (determinism invariant), but the SVG is viewed in
/// Firefox / GitHub READMEs / Confluence where DejaVu is often absent and
/// the font stack falls back to a face of different width; the drawn title
/// then extends past its measured right edge. A fixed [`TITLE_CLEAR_GAP`]
/// alone is eaten by that difference — short titles get their last glyph
/// touched, long ones crossed — so the right-edge clearance is a fixed bump
/// plus a length-derived component ([`TITLE_FALLBACK_PER_W`]). The
/// measurement font cannot change; the gap grows instead.
const TITLE_FALLBACK_PAD: f32 = 12.0;
/// Per-pixel-of-measured-title-width component of the right-edge clearance
/// (see [`title_clear_gap`]): fallback faces differ from the measurement
/// font by a roughly proportional amount, so a longer title needs a
/// proportionally larger margin.
const TITLE_FALLBACK_PER_W: f32 = 0.10;
/// Minimum inset of a left-side title-detour entry from the frame's left
/// edge (see [`title_detour_clear_x`]). The entry sits in the
/// [`FRAME_TITLE_X`] band between the frame's left edge and the title's
/// first glyph; this keeps it clear of the frame border stroke (1.5 px,
/// half inside) with room to spare.
const TITLE_LEFT_ENTRY_MIN: f32 = 4.0;

/// The clearance a title-detour entry keeps past a title's *right* edge: the
/// base gap ([`TITLE_CLEAR_GAP`]) plus a fixed bump and a length-derived
/// component so a viewer's wider fallback font cannot eat the gap (see
/// [`TITLE_FALLBACK_PAD`] / [`TITLE_FALLBACK_PER_W`]).
fn title_clear_gap(title_width: f32) -> f32 {
    TITLE_CLEAR_GAP + TITLE_FALLBACK_PAD + title_width * TITLE_FALLBACK_PER_W
}

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

/// A level item paired with its absolute rect — the obstacle field for M9
/// cross-boundary LCA routing.
type LevelItem = (ItemRef, (f32, f32, f32, f32));

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
        let top = layout_level(
            diagram,
            None,
            diagram.direction,
            &[],
            &Default::default(),
            &Default::default(),
        );
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
    // direct-child node the edge reaches by crossing the frame), and those
    // child nodes themselves. The subgraphs get extra frame padding
    // ([`CROSS_FRAME_PAD`]) so the edge's within-frame stub has room to jog
    // clear of the title and the node's arrowhead; the node set feeds the
    // M10 title-width growth pass in [`layout_level`]. A subgraph reached
    // only through deeper descendants does not qualify: the stub's jog lives
    // in the innermost frame's padding, not this one's.
    let (cross_subs, cross_nodes) = cross_boundary_reach(diagram, &edge_infos, &id_index);

    let top = layout_level(diagram, None, diagram.direction, &edge_infos, &cross_subs, &cross_nodes);
    assemble(diagram, top, &edge_infos)
}

/// The absolute geometry `assemble` works from, shared by the context
/// builder and the routing phase.
struct Geometry<'a> {
    diagram: &'a Diagram,
    id_index: std::collections::HashMap<&'a str, usize>,
    node_rect: std::collections::HashMap<usize, (f32, f32, f32, f32)>,
    frame_rect: std::collections::HashMap<usize, (f32, f32, f32, f32)>,
    /// Immediate children (real-node rects + nested subgraph frame rects, in
    /// absolute coordinates) per subgraph index — used by cross-boundary
    /// routing to detect when a within-frame stub would pierce a sibling and
    /// to route around the frame when it would.
    frame_children: std::collections::HashMap<usize, Vec<(f32, f32, f32, f32)>>,
    direct_pts: std::collections::HashMap<usize, Vec<(f32, f32)>>,
    /// Per-level items (nodes + direct-child frames), in absolute
    /// coordinates — the lane-assignment obstacle field (M9): a lane band
    /// shrinks away from the level's other items.
    level_items: std::collections::HashMap<Option<usize>, Vec<LevelItem>>,
}

impl<'a> Geometry<'a> {
    fn node(&self, gi: usize) -> (f32, f32, f32, f32) {
        self.node_rect.get(&gi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    fn frame(&self, s: usize) -> (f32, f32, f32, f32) {
        self.frame_rect.get(&s).copied().unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    /// A frame's immediate children (node rects + nested frame rects) — the
    /// pierce-test field for within-frame stubs.
    fn children(&self, s: usize) -> Vec<(f32, f32, f32, f32)> {
        self.frame_children.get(&s).cloned().unwrap_or_default()
    }

    /// The rendered width of a subgraph's title (`None` when untitled).
    fn title_width(&self, s: usize) -> Option<f32> {
        self.diagram
            .subgraphs
            .get(s)
            .and_then(|sg| sg.title.as_ref())
            .map(|t| text::measure(t, FRAME_TITLE_FONT_SIZE).width)
    }
}

/// One edge's routing context, derived exactly once (M11.5): endpoint rects,
/// containment chains, representatives, effective sides (the implicit choice
/// vs. the forced override resolved in one place), the flat waypoints for a
/// direct edge, the special-case gates, and — after the separation pre-
/// passes — the (possibly fanned) ports and the assigned jog lane. Before
/// this hoist, `assemble` derived all of this twice (pre-pass + routing
/// loop) and kept the two copies in agreement by hand.
struct EdgeCtx {
    ei: usize,
    from_id: String,
    to_id: String,
    is_direct: bool,
    is_self_loop: bool,
    /// Global endpoint node indices.
    fi: usize,
    ti: usize,
    from_rect: (f32, f32, f32, f32),
    to_rect: (f32, f32, f32, f32),
    /// Frames crossed on each side, innermost first, ending with the
    /// representative at the LCA level (empty = the endpoint is a direct
    /// child of the LCA and is its own representative).
    from_chain: Vec<usize>,
    to_chain: Vec<usize>,
    /// The LCA level that owns the edge (`None` = top level), its effective
    /// direction, and the flow axis that follows from it.
    lca: Option<usize>,
    lca_dir: Direction,
    flow: FlowAxis,
    /// Forced page-space sides (`from=` / `to=`); `None` keeps the implicit
    /// choice (M11).
    forced_from: Option<Side>,
    forced_to: Option<Side>,
    /// The implicit side choice (what the flat engine / [`sides_along`] pick).
    implicit_from: Side,
    implicit_to: Side,
    /// The effective sides (forced override applied — literal semantics).
    from_side: Side,
    to_side: Side,
    /// The endpoints' representatives at the LCA level with their rects (a
    /// rep frame's rect, or the endpoint node's own rect when the chain is
    /// empty).
    rep_from: ItemRef,
    rep_to: ItemRef,
    rep_from_rect: (f32, f32, f32, f32),
    rep_to_rect: (f32, f32, f32, f32),
    /// The flat engine's waypoints (direct edges only; empty otherwise).
    raw: Vec<(f32, f32)>,
    /// Around-target gate (M5 follow-up), decided once here on center
    /// geometry — the same test the candidate generator runs — so the
    /// port-separation exclusion below and the generator agree by
    /// construction rather than by hand-maintained copies: the straight
    /// within-frame target stub (at the target node's center cross) would
    /// pierce a sibling of the target inside its frame. A forced `to` side
    /// disables it (the forced side is honored literally, and the forced
    /// stub's own sibling-avoidance ladder takes over).
    around_target: bool,
    /// Filled after the separation pre-passes: the endpoint nodes'
    /// (possibly fanned) ports and their cross-coordinates, and the cross-
    /// boundary edge's assigned jog lane in its LCA gap.
    from_port: (f32, f32),
    to_port: (f32, f32),
    from_cross: f32,
    to_cross: f32,
    lane: f32,
}

/// Derive one edge's [`EdgeCtx`] from the laid-out geometry.
fn edge_context(geom: &Geometry, edge_infos: &[EdgeInfo], ei: usize) -> EdgeCtx {
    let diagram = geom.diagram;
    let e = &diagram.edges[ei];
    let info = edge_infos.get(ei);
    let lca = info.map(|i| i.lca).unwrap_or(None);
    let lca_dir = effective_direction(lca, diagram);
    let fi = geom.id_index[e.from.as_str()];
    let ti = geom.id_index[e.to.as_str()];
    let from = geom.node(fi);
    let to = geom.node(ti);
    let is_direct = info.map(|i| i.is_direct).unwrap_or(true);
    // M11: forced page-space sides (`from=` / `to=`); `None` keeps the
    // implicit choice.
    let forced_from = e.from_side.map(side_of_edge);
    let forced_to = e.to_side.map(side_of_edge);
    let (raw, from_chain, to_chain, implicit_from, implicit_to, rep_from_rect, rep_to_rect) =
        if is_direct {
            let raw = geom.direct_pts.get(&ei).cloned().unwrap_or_default();
            let fs = raw
                .first()
                .map(|&p| side_of_port(from, p))
                .unwrap_or(Side::Bottom);
            let ts = raw
                .last()
                .map(|&p| side_of_port(to, p))
                .unwrap_or(Side::Top);
            (raw, Vec::new(), Vec::new(), fs, ts, from, to)
        } else {
            let from_chain = chain_to_lca(diagram.nodes[fi].group, lca, &diagram.subgraphs);
            let to_chain = chain_to_lca(diagram.nodes[ti].group, lca, &diagram.subgraphs);
            let rf = from_chain.last().map(|&s| geom.frame(s)).unwrap_or(from);
            let rt = to_chain.last().map(|&s| geom.frame(s)).unwrap_or(to);
            let (exit, entry) = sides_along(lca_dir, center(rf), center(rt));
            (Vec::new(), from_chain, to_chain, exit, entry, rf, rt)
        };
    let from_side = forced_from.unwrap_or(implicit_from);
    let to_side = forced_to.unwrap_or(implicit_to);
    let around_target = forced_to.is_none()
        && !is_direct
        && to_chain.len() == 1
        && from_chain.len() <= 1
        && {
            let sibs: Vec<(f32, f32, f32, f32)> = geom
                .children(to_chain[0])
                .into_iter()
                .filter(|r| !rects_near(*r, to))
                .collect();
            let cc = cross_of(center(to), to_side);
            let crp = port_at_cross(rep_to_rect, to_side, cc);
            let ctp = port(to, to_side);
            sibs.iter().any(|sib| segment_intersects_rect(crp, ctp, *sib))
        };
    EdgeCtx {
        ei,
        from_id: e.from.clone(),
        to_id: e.to.clone(),
        is_direct,
        is_self_loop: is_direct && e.from == e.to,
        fi,
        ti,
        from_rect: from,
        to_rect: to,
        from_chain,
        to_chain,
        lca,
        lca_dir,
        flow: FlowAxis::from_direction(lca_dir),
        forced_from,
        forced_to,
        implicit_from,
        implicit_to,
        from_side,
        to_side,
        rep_from: info.map(|i| i.rep_from).unwrap_or(ItemRef::Node(fi)),
        rep_to: info.map(|i| i.rep_to).unwrap_or(ItemRef::Node(ti)),
        rep_from_rect,
        rep_to_rect,
        raw,
        around_target,
        from_port: (0.0, 0.0),
        to_port: (0.0, 0.0),
        from_cross: 0.0,
        to_cross: 0.0,
        lane: 0.0,
    }
}

/// Turn a top-level [`LevelOut`] (page coordinates) into the public
/// [`Layout`]. The routing regime is unified (M11.5): every edge's context
/// is built once, the M9 separation pre-passes run over the contexts, and
/// then every edge — direct, cross-boundary, forced, self-loop — routes
/// through the one dispatch point ([`route_edge`]) against the one obstacle
/// world, in a deterministic priority (forced edges first, then declaration
/// order), with already-routed edges' segments joining the world as
/// obstacles for the edges routed after them.
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

    let mut level_items: HashMap<Option<usize>, Vec<LevelItem>> = HashMap::new();
    for (gi, nd) in diagram.nodes.iter().enumerate() {
        let r = node_rect.get(&gi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
        level_items.entry(nd.group).or_default().push((ItemRef::Node(gi), r));
    }
    for (si, sg) in diagram.subgraphs.iter().enumerate() {
        let r = frame_rect.get(&si).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
        level_items.entry(sg.parent).or_default().push((ItemRef::Sub(si), r));
    }

    let geom = Geometry {
        diagram,
        id_index,
        node_rect,
        frame_rect,
        frame_children,
        direct_pts,
        level_items,
    };

    // ---- M11.5: build each edge's route context once ----------------------
    let mut ctxs: Vec<EdgeCtx> = (0..diagram.edges.len())
        .map(|ei| edge_context(&geom, edge_infos, ei))
        .collect();

    // ---- M9 pre-pass 1: port separation -----------------------------------
    // Gather, for every edge endpoint that touches a node, the node side it
    // uses (already resolved in the context: forced override or implicit
    // choice) and the other endpoint's centre cross-coordinate, so sides
    // shared by more than one edge can fan their ports apart.
    let mut port_ends: Vec<PortEnd> = Vec::new();
    for ctx in &ctxs {
        port_ends.push(PortEnd {
            edge: ctx.ei,
            is_from: true,
            node_idx: ctx.fi,
            node_rect: ctx.from_rect,
            side: ctx.from_side,
            other_cross: cross_of(center(ctx.to_rect), ctx.from_side),
        });
        // An around-target-frame edge enters its target from a perpendicular
        // side (not the one [`sides_along`] picks), so its target port is not
        // on this side; exclude it from target-side port separation, which
        // would otherwise move the entry point and suppress the around-
        // route. (The gate was decided in the context, on center geometry.)
        if !ctx.around_target {
            port_ends.push(PortEnd {
                edge: ctx.ei,
                is_from: false,
                node_idx: ctx.ti,
                node_rect: ctx.to_rect,
                side: ctx.to_side,
                other_cross: cross_of(center(ctx.from_rect), ctx.to_side),
            });
        }
    }
    let sep = separate_ports(&port_ends);

    // ---- M9 pre-pass 2: lane separation ----------------------------------
    let mut lane_edges: Vec<LaneEdge> = Vec::new();
    for ctx in &ctxs {
        if ctx.is_direct {
            continue;
        }
        let from_cross = sep
            .get(&(ctx.ei, true))
            .copied()
            .unwrap_or(cross_of(center(ctx.rep_from_rect), ctx.from_side));
        // Band obstacles for the lane group: the source's peers at the LCA
        // level (everything except the source rep). Targets sit at the gap's
        // far end, outside the gap, so they do not intrude on the lane band;
        // excluding only the source keeps this set shared across a gap group
        // (whose edges share a source but may differ in target).
        let obstacles: Vec<(f32, f32, f32, f32)> = geom
            .level_items
            .get(&ctx.lca)
            .into_iter()
            .flatten()
            .filter(|(it, _)| *it != ctx.rep_from)
            .map(|(_, r)| *r)
            .collect();
        lane_edges.push(LaneEdge {
            edge: ctx.ei,
            rep_from: ctx.rep_from,
            from_side: ctx.from_side,
            rep_to: ctx.rep_to,
            // Flow coordinates of the two rep ports ([`port_flow`], not
            // [`side_flow_coord`]: a forced side can be perpendicular to the
            // flow axis, in which case the port's own flow coordinate is the
            // right band bound).
            from_flow: port_flow(ctx.rep_from_rect, ctx.from_side, from_cross, ctx.flow),
            to_flow: port_flow(
                ctx.rep_to_rect,
                ctx.to_side,
                cross_of(center(ctx.rep_to_rect), ctx.to_side),
                ctx.flow,
            ),
            from_cross,
            axis: ctx.flow,
            obstacles,
        });
    }
    let lanes = assign_lanes(&lane_edges);

    // Fill the per-edge ports and lanes from the pre-pass results.
    for ctx in ctxs.iter_mut() {
        let fc = sep.get(&(ctx.ei, true)).copied();
        let tc = sep.get(&(ctx.ei, false)).copied();
        if ctx.is_direct {
            // Apply the separated cross-coordinates onto the flat waypoints'
            // ports (the side each end sits on is read off the point, as the
            // pre-pass did).
            if let Some(c) = fc
                && let Some(&p) = ctx.raw.first()
            {
                let side = side_of_port(ctx.from_rect, p);
                ctx.raw[0] = set_cross(p, side, c);
            }
            if let Some(c) = tc
                && let Some(p) = ctx.raw.last().copied()
            {
                let side = side_of_port(ctx.to_rect, p);
                let n = ctx.raw.len();
                ctx.raw[n - 1] = set_cross(p, side, c);
            }
        }
        ctx.from_port = port_at_cross(
            ctx.from_rect,
            ctx.from_side,
            fc.unwrap_or_else(|| cross_of(center(ctx.from_rect), ctx.from_side)),
        );
        ctx.to_port = port_at_cross(
            ctx.to_rect,
            ctx.to_side,
            tc.unwrap_or_else(|| cross_of(center(ctx.to_rect), ctx.to_side)),
        );
        ctx.from_cross = cross_of(ctx.from_port, ctx.from_side);
        ctx.to_cross = cross_of(ctx.to_port, ctx.to_side);
        if !ctx.is_direct {
            ctx.lane = lanes.get(&ctx.ei).copied().unwrap_or_else(|| {
                jog_flow(
                    port_flow(ctx.rep_from_rect, ctx.from_side, ctx.from_cross, ctx.flow),
                    port_flow(ctx.rep_to_rect, ctx.to_side, ctx.to_cross, ctx.flow),
                )
            });
        }
    }

    // ---- Routing phase: one pass, deterministic priority (M11.5) ----------
    // Forced edges route first (their excursions claim space first), then
    // declaration order; each routed edge's segments join the world, so peer
    // segments are obstacles for every later edge — route quality no longer
    // depends on an edge happening to be forced.
    let mut order: Vec<usize> = (0..diagram.edges.len()).collect();
    order.sort_by_key(|&ei| {
        let forced = ctxs[ei].forced_from.is_some() || ctxs[ei].forced_to.is_some();
        (!forced, ei)
    });

    let mut routed_segments: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut edges_out: Vec<Option<EdgePath>> = (0..diagram.edges.len()).map(|_| None).collect();
    for &ei in &order {
        let world = edge_world(&geom, &ctxs[ei], &routed_segments);
        let pts = route_edge(&ctxs[ei], &world, &geom, (top.w, top.h));
        for w in pts.windows(2) {
            routed_segments.push(segment_rect(w[0], w[1], SEGMENT_OBSTACLE_PAD));
        }
        edges_out[ei] = Some(EdgePath {
            from: ctxs[ei].from_id.clone(),
            to: ctxs[ei].to_id.clone(),
            points: pts,
        });
    }
    let mut edges_out: Vec<EdgePath> = edges_out
        .into_iter()
        .map(|e| e.expect("every edge routed"))
        .collect();

    let mut nodes_out = Vec::with_capacity(diagram.nodes.len());
    for (gi, n) in diagram.nodes.iter().enumerate() {
        let (x, y, w, h) = geom.node(gi);
        nodes_out.push(NodeRect {
            id: n.id.clone(),
            x,
            y,
            w,
            h,
        });
    }

    let mut subgraphs_out = Vec::with_capacity(diagram.subgraphs.len());
    for gi in 0..diagram.subgraphs.len() {
        let (x, y, w, h) = geom.frame(gi);
        subgraphs_out.push(SubgraphRect {
            index: gi,
            x,
            y,
            w,
            h,
        });
    }

    // ---- M11: canvas growth for forced-side excursions --------------------
    // A forced side can route outside the laid-out content bounds (an
    // excursion around a frame at the canvas edge, or an escape past the
    // page margin). Grow — and, for negative excursions, translate — the
    // canvas so every drawn point lies inside it; the renderer's viewBox
    // comes straight from width/height. For an unforced diagram this is a
    // no-op: content sits exactly [`MARGIN`] inside the canvas on every
    // side, so nothing pokes out and the extents match to within float
    // noise (hence the 1e-2 tolerance).
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for n in &nodes_out {
        min_x = min_x.min(n.x);
        min_y = min_y.min(n.y);
        max_x = max_x.max(n.x + n.w);
        max_y = max_y.max(n.y + n.h);
    }
    for s in &subgraphs_out {
        min_x = min_x.min(s.x);
        min_y = min_y.min(s.y);
        max_x = max_x.max(s.x + s.w);
        max_y = max_y.max(s.y + s.h);
    }
    for e in &edges_out {
        for &(px, py) in &e.points {
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
    }
    if min_x < 0.0 || min_y < 0.0 {
        let sx = if min_x < 0.0 { -min_x } else { 0.0 };
        let sy = if min_y < 0.0 { -min_y } else { 0.0 };
        for n in nodes_out.iter_mut() {
            n.x += sx;
            n.y += sy;
        }
        for s in subgraphs_out.iter_mut() {
            s.x += sx;
            s.y += sy;
        }
        for e in edges_out.iter_mut() {
            for p in e.points.iter_mut() {
                p.0 += sx;
                p.1 += sy;
            }
        }
        max_x += sx;
        max_y += sy;
    }
    let width = if max_x + MARGIN > top.w + 1e-2 {
        max_x + MARGIN
    } else {
        top.w
    };
    let height = if max_y + MARGIN > top.h + 1e-2 {
        max_y + MARGIN
    } else {
        top.h
    };

    Layout {
        nodes: nodes_out,
        edges: edges_out,
        subgraphs: subgraphs_out,
        width,
        height,
    }
}

/// The one obstacle world this edge routes against (M11.5): every node
/// interior — including its own endpoints, which a route may touch only at
/// a designated port (so it cannot cut back through either box) — the frame
/// outlines at its LCA level as thin bands (crossable only at designated
/// waypoints: a rep port or a recorded stub crossing), and the segments of
/// every already-routed edge.
fn edge_world(
    geom: &Geometry,
    ctx: &EdgeCtx,
    routed: &[(f32, f32, f32, f32)],
) -> Obstacles {
    let mut obs = Obstacles::empty();
    for (gi, _) in geom.diagram.nodes.iter().enumerate() {
        obs.push_node(geom.node(gi));
    }
    for &(idx, r) in geom
        .level_items
        .get(&ctx.lca)
        .into_iter()
        .flatten()
    {
        if matches!(idx, ItemRef::Sub(_)) {
            obs.push_frame(r);
        }
    }
    for &r in routed {
        obs.segments.push(r);
    }
    obs
}

/// Route one edge: the single dispatch point (M11.5). Candidate generators
/// run in priority order — each decides for itself whether it applies — and
/// the first candidate whose polyline clears the obstacle world wins; the
/// last candidate is terminal (kept unconditionally: its internal ladder
/// already ends in a route that keeps the ports — the bounded-ladder
/// convention). The special cases keep their fallbacks; the point of the
/// hoist is one dispatch, not fewer cases.
fn route_edge(
    ctx: &EdgeCtx,
    world: &Obstacles,
    geom: &Geometry,
    canvas: (f32, f32),
) -> Vec<(f32, f32)> {
    let mut candidates: Vec<Vec<(f32, f32)>> = Vec::new();
    if ctx.is_self_loop {
        // A self-loop: a forced side gets real loop routing (M11) — the
        // user's explicit shape request; an unforced loop keeps the flat
        // engine's below-node stub, orthogonalized. Either way, terminal.
        if ctx.forced_from.is_some() || ctx.forced_to.is_some() {
            candidates.push(self_loop_path(
                ctx.from_rect,
                ctx.from_side,
                ctx.to_side,
                ctx.from_cross,
                ctx.to_cross,
                canvas.0,
                canvas.1,
            ));
        } else {
            candidates.push(ortho_chain(&ctx.raw, ctx.flow));
        }
    } else if ctx.is_direct {
        let raw_fs = ctx.raw.first().map(|&p| side_of_port(ctx.from_rect, p));
        let raw_ts = ctx.raw.last().map(|&p| side_of_port(ctx.to_rect, p));
        let contradicts = ctx.raw.len() >= 2
            && (ctx.forced_from.is_some() || ctx.forced_to.is_some())
            && (ctx.forced_from != raw_fs || ctx.forced_to != raw_ts);
        if contradicts {
            // M11: a forced side contradicts the implicit choice — reroute
            // the whole edge honoring the forced sides literally (the port
            // lands on the requested side, no reversion). Each port escapes
            // outward along its side; the [`route_lca`] ladder finds a
            // route that keeps the ports. Terminal.
            candidates.push(forced_direct_path(
                ctx.from_port,
                ctx.from_side,
                ctx.to_port,
                ctx.to_side,
                ctx.flow,
                world,
            ));
        } else {
            // M11.5: the same ladder every edge uses — each flat segment is
            // a simple Z when the world is clear of it, rerouted through the
            // track graph when not, the plain Z as the last resort. This
            // closes the pass-through-a-node hole the blind orthogonalizer
            // had. Terminal.
            candidates.push(direct_chain_route(&ctx.raw, ctx.flow, world));
        }
    } else {
        // Cross-boundary: the around-target-frame candidate first (it
        // checks siblings and the obstacle world inside its generator),
        // then the frame-aware pieces (terminal).
        if let Some(route) = around_target_candidate(ctx, geom, world) {
            candidates.push(route);
        }
        candidates.push(cross_pieces_candidate(ctx, geom, world));
    }
    candidates
        .iter()
        .find(|p| world.clear(p))
        .unwrap_or_else(|| candidates.last().expect("at least one candidate"))
        .clone()
}

/// Route a direct internal edge through the same ladder every edge uses
/// (M11.5): each consecutive pair of flat waypoints is a simple Z at the
/// pair's flow midpoint — exactly what the blind orthogonalizer produced —
/// but the Z is now clearance-checked against the obstacle world and, when
/// blocked (a peer node in the corridor, a frame outline, an already-routed
/// edge), rerouted through the track graph; the plain Z remains the last
/// resort. The flat waypoints (dummy centers, fanned ports) are preserved.
fn direct_chain_route(raw: &[(f32, f32)], axis: FlowAxis, obs: &Obstacles) -> Vec<(f32, f32)> {
    if raw.len() < 2 {
        return raw.to_vec();
    }
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(raw.len() * 2);
    out.push(raw[0]);
    for w in raw.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let (c0, f0) = cross_flow(p0, axis);
        let (c1, f1) = cross_flow(p1, axis);
        if (c0 - c1).abs() < 1e-3 || (f0 - f1).abs() < 1e-3 {
            // Already axis-aligned along the flow or cross axis: keep
            // straight (waypoints preserved).
            out.push(p1);
        } else {
            let jf = jog_flow(f0, f1);
            out.extend(route_lca(p0, p1, axis, jf, obs));
        }
    }
    dedup_consecutive(&mut out);
    out
}

/// The around-target-frame candidate (M5 follow-up, hoisted to the
/// dispatch): applies when the context's `around_target` gate fired — the
/// straight within-frame target stub would pierce a sibling — and yields
/// the first of the two perpendicular sides whose route clears both the
/// siblings and the obstacle world. `None` falls back to the frame-aware
/// pieces (the straight stub).
fn around_target_candidate(
    ctx: &EdgeCtx,
    geom: &Geometry,
    world: &Obstacles,
) -> Option<Vec<(f32, f32)>> {
    if !ctx.around_target {
        return None;
    }
    let sibs: Vec<(f32, f32, f32, f32)> = geom
        .children(ctx.to_chain[0])
        .into_iter()
        .filter(|r| !rects_near(*r, ctx.to_rect))
        .collect();
    try_around_target_route(
        ctx.from_rect,
        ctx.to_rect,
        ctx.rep_to_rect,
        &ctx.from_chain,
        ctx.flow,
        ctx.from_side,
        ctx.to_side,
        &sibs,
        &|s| geom.frame(s),
        ctx.from_port,
        ctx.lane,
        world,
    )
}

/// The frame-aware pieces candidate (M5/M7/M10/M11 logic, unchanged):
/// within-frame stubs (title detours, forced-stub sibling avoidance, nested
/// frame crossings) plus the LCA segment routed through the same ladder
/// ([`route_lca`]). Terminal: the ladder's last resort keeps the ports.
fn cross_pieces_candidate(
    ctx: &EdgeCtx,
    geom: &Geometry,
    world: &Obstacles,
) -> Vec<(f32, f32)> {
    let cp = cross_boundary_path(
        ctx.from_rect,
        ctx.to_rect,
        &ctx.from_chain,
        &ctx.to_chain,
        ctx.lca_dir,
        ctx.from_port,
        ctx.to_port,
        ctx.from_cross,
        ctx.to_cross,
        ctx.from_side,
        ctx.to_side,
        ctx.forced_from.is_some(),
        ctx.forced_to.is_some(),
        &|s| geom.frame(s),
        &|s| geom.title_width(s),
        &|s| geom.children(s),
    );
    let CrossPieces { src_stub, lca, tgt_stub } = cp;
    let mut pts = Vec::new();
    pts.extend(ortho_chain(&src_stub, ctx.flow));
    pts.extend(lca_segment(
        lca[0],
        ctx.from_side,
        lca[1],
        ctx.to_side,
        ctx.flow,
        ctx.lane,
        world,
    ));
    pts.extend(ortho_chain(&tgt_stub, ctx.flow));
    dedup_consecutive(&mut pts);
    pts
}

/// Is `side` perpendicular to the flow axis (a frame's side border under a
/// vertical flow, or its top/bottom under a horizontal one)? Only a forced
/// side can be — the implicit choice always lies along the flow axis.
fn side_perpendicular(side: Side, axis: FlowAxis) -> bool {
    match axis {
        FlowAxis::Vertical => matches!(side, Side::Left | Side::Right),
        FlowAxis::Horizontal => matches!(side, Side::Top | Side::Bottom),
    }
}

/// The LCA-level segment between the two rep-frame ports, routed through
/// the ladder. A forced side perpendicular to the flow axis puts the port
/// on a frame's side border, where a flow-axis first leg would ride along
/// the outline — such a port escapes outward first ([`SIDE_ESCAPE`], the
/// M11 escape convention) and the port legs are attached around the routed
/// part. An implicit side always lies along the flow axis, so its port
/// needs no escape (its first leg already leaves the frame perpendicular).
fn lca_segment(
    a: (f32, f32),
    a_side: Side,
    b: (f32, f32),
    b_side: Side,
    axis: FlowAxis,
    lane: f32,
    obs: &Obstacles,
) -> Vec<(f32, f32)> {
    let a_perp = side_perpendicular(a_side, axis);
    let b_perp = side_perpendicular(b_side, axis);
    let a_out = if a_perp { escape_point(a, a_side, SIDE_ESCAPE) } else { a };
    let b_out = if b_perp { escape_point(b, b_side, SIDE_ESCAPE) } else { b };
    let mut pts = route_lca(a_out, b_out, axis, lane, obs);
    if a_perp {
        pts.insert(0, a_out);
        pts.insert(0, a);
    }
    if b_perp {
        pts.push(b_out);
        pts.push(b);
    }
    dedup_consecutive(&mut pts);
    pts
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
    cross_nodes: &std::collections::HashSet<usize>,
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
        let out = layout_level(
            diagram,
            Some(cs),
            child_dir,
            edge_infos,
            cross_subs,
            cross_nodes,
        );
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
        // The frame is at least wide enough for its contents plus side
        // padding, but no narrower than its title: the title starts at
        // [`FRAME_TITLE_X`] and gets the same clearance past its last glyph.
        let title_w = diagram.subgraphs[cs]
            .title
            .as_ref()
            .map(|t| text::measure(t, FRAME_TITLE_FONT_SIZE).width)
            .unwrap_or(0.0);
        let mut frame_w = (child.out.w + 2.0 * FRAME_PAD_X)
            .max(title_w + 2.0 * FRAME_TITLE_X);
        // M10: when the title determines the frame width, the margin past the
        // title's right edge is only [`FRAME_TITLE_X`] — smaller than
        // [`FRAME_PAD_X`] — so a title detour's entry would clamp *inside*
        // the title and the detour gives up (the stub crosses the title).
        // Mirror the [`CROSS_FRAME_PAD`] trick horizontally: when a
        // cross-boundary edge reaches an immediate child that sits under the
        // title's right half (where the detour enters — one under the left
        // half uses the left-side entry, which needs no extra room), grow the
        // frame's width so the clear band past the title exists. Growing
        // sideways keeps the children anchored at the left/top insets, so
        // the group's internal arrangement is untouched (the M5 guarantee).
        // Like [`CROSS_FRAME_PAD`] this over-approximates: the growth does
        // not know which side the edge will enter on, so a frame may widen
        // for a detour that never arises.
        if diagram.subgraphs[cs].title.is_some() {
            let right_gap = title_clear_gap(title_w);
            let title_x1 = FRAME_TITLE_X + title_w;
            let need = title_x1 + right_gap + FRAME_PAD_X;
            let title_mid = FRAME_TITLE_X + title_w / 2.0;
            let under_right_half = child.out.nodes.iter().any(|&(gi, nx, _, nw, _)| {
                // Node center x in frame-local coordinates: the child's
                // contents are offset into the frame by `inner_ox`.
                let ncx = FRAME_PAD_X + nx + nw / 2.0;
                cross_nodes.contains(&gi)
                    && ncx >= title_mid
                    && ncx <= title_x1 + right_gap
            });
            if under_right_half && frame_w < need {
                frame_w = need;
            }
        }
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
/// frame — and the set of those child nodes themselves. Such frames get
/// extra padding ([`CROSS_FRAME_PAD`]) so the edge's within-frame stub has
/// room to jog clear of the title and the node's arrowhead, and (M10) the
/// frame-growth pass in [`layout_level`] widens a title-sized frame so a
/// title detour has an entry. A subgraph reached only through deeper
/// descendants does not qualify: the stub's jog lives in the innermost
/// frame's padding, not this one's.
///
/// `id_index` maps node ids to their index in `diagram.nodes`. An endpoint
/// qualifies a subgraph `S` when the endpoint is a direct child of `S`
/// (`node.group == Some(S)`) and the edge's LCA is a strict ancestor of `S`
/// (`lca != Some(S)`) — i.e. the edge leaves `S` to reach the rest of the
/// graph, rather than staying internal to `S`.
#[allow(clippy::type_complexity)]
fn cross_boundary_reach(
    diagram: &Diagram,
    edge_infos: &[EdgeInfo],
    id_index: &std::collections::HashMap<&str, usize>,
) -> (
    std::collections::HashSet<usize>,
    std::collections::HashSet<usize>,
) {
    let mut subs = std::collections::HashSet::new();
    let mut nodes = std::collections::HashSet::new();
    for (ei, e) in diagram.edges.iter().enumerate() {
        let Some(info) = edge_infos.get(ei) else { continue; };
        for nid in [e.from.as_str(), e.to.as_str()].map(|id| id_index[id]) {
            if let Some(group) = diagram.nodes[nid].group
                && info.lca != Some(group) {
                    subs.insert(group);
                    nodes.insert(nid);
                }
        }
    }
    (subs, nodes)
}

/// `(top_inset, bottom_padding)` for a child subgraph's frame, adding the
/// cross-boundary extra padding ([`CROSS_FRAME_PAD`]) to both when `cross`
/// is true (the subgraph has a cross-boundary edge to an immediate child).
/// Both insets grow along the LCA-level flow axis, where the within-frame
/// stubs jog, so the extra room clears the title (above the node) and the
/// node's arrowhead (below the jog).
///
/// The top inset is a one-line title band ([`FRAME_TITLE_H`]) plus one
/// [`text::line_height`] per *additional* title line, so a multi-line
/// title's stacked lines stay inside the band.
fn frame_insets(sg: &Subgraph, cross: bool) -> (f32, f32) {
    let top = match &sg.title {
        Some(t) => {
            FRAME_TITLE_H
                + (text::line_count(t) - 1) as f32 * text::line_height(FRAME_TITLE_FONT_SIZE)
        }
        None => FRAME_PAD_Y,
    };
    if cross {
        (top + CROSS_FRAME_PAD, FRAME_PAD_Y + CROSS_FRAME_PAD)
    } else {
        (top, FRAME_PAD_Y)
    }
}

// ============ Cross-boundary edge routing (M5) ============

/// One of the four sides of an axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
/// title text the stub would cross the title; this returns an entry `x` in
/// the clear part of the top band so the stub can dive in there and jog
/// across to the node below the title. Two entries exist:
///
/// * just past the title's **right** edge, at
///   `title_x1 + title_clear_gap(title_width)` — the clearance includes the
///   fixed + proportional fallback margins ([`title_clear_gap`]) because the
///   drawn title can extend past its measured right edge in a viewer whose
///   font stack falls back to a wider face;
/// * just **left** of the title's first glyph, at
///   `title_x0 - TITLE_CLEAR_GAP` — the title's left edge is anchored at
///   [`FRAME_TITLE_X`] by the renderer, so no font fallback can widen it
///   leftward and the small base gap is safe there.
///
/// The entry nearer the node (shorter jog) wins among the usable ones: a
/// node under the title's left half enters left of the title, one under its
/// right half past the right edge. `None` is returned — meaning "stay
/// straight, no detour" — when the node's `x` is already clear of the title,
/// or when neither entry fits (no room past the title and no room left of
/// it): the frame-growth pass in [`layout_level`] should have widened the
/// frame to make the right entry fit for any cross-boundary endpoint that
/// gets here, so `None` is a last-resort fallback (the stub then crosses the
/// title, no worse than the previous frame-center design which crossed it at
/// the center).
fn title_detour_clear_x(rep: (f32, f32, f32, f32), node_cross: f32, title_width: f32) -> Option<f32> {
    let (rx, _ry, rw, _rh) = rep;
    let title_x0 = rx + FRAME_TITLE_X;
    let title_x1 = rx + FRAME_TITLE_X + title_width;
    let right_gap = title_clear_gap(title_width);
    // Node already clear of the title text (to either side)? Stay straight.
    if node_cross < title_x0 - TITLE_CLEAR_GAP || node_cross > title_x1 + right_gap {
        return None;
    }
    let right_x = title_x1 + right_gap;
    let left_x = title_x0 - TITLE_CLEAR_GAP;
    // A candidate is usable when it lands inside the frame: the right entry
    // keeps [`FRAME_PAD_X`] of side padding (which the frame-growth pass
    // guarantees by growing the width when the title determines it), the
    // left entry keeps [`TITLE_LEFT_ENTRY_MIN`] inside the border.
    let right_ok = right_x <= rx + rw - FRAME_PAD_X + 1e-3;
    let left_ok = left_x >= rx + TITLE_LEFT_ENTRY_MIN;
    // Prefer the nearer side: a node under the title's left half jogs less
    // from a left entry, one under the right half from a right entry.
    let prefer_left = node_cross < (title_x0 + title_x1) / 2.0;
    for (x, ok) in if prefer_left {
        [(left_x, left_ok), (right_x, right_ok)]
    } else {
        [(right_x, right_ok), (left_x, left_ok)]
    } {
        if ok {
            return Some(x);
        }
    }
    None
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
/// (Liang–Barsky line clipping; the interval form is
/// [`segment_rect_interval`].) Used to detect when a cross-boundary
/// within-frame stub would pierce a sibling, to reject an around-frame
/// route whose perpendicular entry would cut across a sibling, and by the
/// obstacle-world clearance checks.
fn segment_intersects_rect(
    p0: (f32, f32),
    p1: (f32, f32),
    r: (f32, f32, f32, f32),
) -> bool {
    segment_rect_interval(p0, p1, r).is_some_and(|(t0, t1)| t0 < t1 - 1e-6)
}

fn segment_rect_interval(
    p0: (f32, f32),
    p1: (f32, f32),
    r: (f32, f32, f32, f32),
) -> Option<(f32, f32)> {
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
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                if t > t0 {
                    t0 = t;
                }
            } else {
                if t < t0 {
                    return None;
                }
                if t < t1 {
                    t1 = t;
                }
            }
        }
    }
    if t0 <= t1 + 1e-9 {
        Some((t0, t1))
    } else {
        None
    }
}

/// The `[t0, t1]` parameter interval of the segment `p0`→`p1` lying inside
/// the closed axis-aligned rect `r` (Liang–Barsky clipping), or `None` when
/// the segment misses `r` entirely. A segment merely touching the rect
/// (degenerate interval, `t0 == t1`) yields `Some` — callers that mean
/// "passes through" use [`segment_intersects_rect`], which requires a
/// positive-length overlap; the obstacle-world checks use the raw interval
/// to reason about *where* along the segment a rect is touched.

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

// ============ M9 — edge separation & obstacle-aware LCA routing ============
//
// Two failure modes the M7 midpoint-jog orthogonalizer could not avoid:
//
// 1. An edge passing *through* a node. The LCA-level segment of a cross-
//    boundary edge runs between two representatives' ports; its midpoint jog
//    can land inside a third node that sits between them (e.g. `User --> VPN`
//    whose jog crosses `prd`, making it look as if `prd` connects to `VPN`).
// 2. Parallel edges laying *on top of* each other. Several edges from one
//    source to one target subgraph share the source's single port and the
//    same jog lane, so their trunks overlap and their labels collide (e.g.
//    the five `VPN --> cvm` edges, and `lb --> api1/api2` in `infra`).
//
// M9 adds, for cross-boundary LCA segments (and direct-edge endpoints):
//   - **Port separation** — a node side carrying more than one edge fans the
//     ports across the side (ordered by the other endpoint's position), so
//     edges no longer all leave/enter at the centre. Single-edge sides keep
//     the centre (the prior, tested behaviour).
//   - **Lane separation** — parallel LCA segments sharing a source, side, and
//     target representative run at distinct jog flow-coordinates in their
//     gap, fanning without overlapping.
//   - **Obstacle avoidance** — when an LCA segment's jog would cross a peer
//     node, it is rerouted around it along a rectilinear track graph (Dijkstra
//     over obstacle-edge tracks), so no edge passes through a node.
//
// Within-frame stubs (the frame-aware short runs from a node to its rep frame
// port) keep the M5/M7 logic unchanged; only the inter-representative LCA
// segment is rerouted. The M5 headline guarantee (a subgraph's internal
// layout is never disturbed by a crossing edge) is preserved.

/// Clearance kept between a routed LCA segment and any peer node/frame rect
/// it must avoid (the track graph routes along the obstacle edges offset by
/// this much; the simple-Z clear check inflates obstacles by it).
const ROUTE_PAD: f32 = 6.0;
/// Track-graph tracks are placed this far beyond each obstacle edge — a touch
/// more than [`ROUTE_PAD`] so a route running along a track clears the
/// [`ROUTE_PAD`]-inflated obstacle check (a segment exactly on an inflated
/// edge reads as touching it).
const TRACK_OFF: f32 = ROUTE_PAD + 2.0;
/// A node port is inset this far from the corners of its side when several
/// edges share the side, so fanned ports stay clear of the box corners.
const PORT_INSET: f32 = 7.0;
/// Minimum cross-coordinate spacing between fanned ports on the same side
/// (and between parallel jog lanes), so adjacent edges read as distinct.
const FAN_SEP: f32 = 10.0;
/// A jog lane is inset this far from the edges of the gap it runs in.
const LANE_INSET: f32 = 5.0;
/// Bend penalty added per turn in the track-graph route, so a shortest path
/// with fewer right-angle bends is preferred.
const BEND_PENALTY: f32 = 12.0;
/// How far a forced-side route escapes outward from a port (along the
/// side's outward normal) before orthogonal routing begins (M11). Sized to
/// clear the arrowhead's back-extent (`markerHeight`, 10) comfortably, and
/// — being just past the top-level page margin ([`MARGIN`], 20) — to push a
/// forced excursion at the canvas edge outside it, where the canvas-growth
/// pass in [`assemble`] grows the viewBox instead of silently clipping.
const SIDE_ESCAPE: f32 = 24.0;
/// How far a forced self-loop's excursion runs beyond the node's sides
/// (M11), before wrapping around to the entry side.
const LOOP_OUT: f32 = 18.0;
/// Thickness each side of an already-routed edge segment when it is turned
/// into an obstacle for a later forced route (M11). Half the fan spacing
/// ([`FAN_SEP`]) — enough that a parallel edge cannot lay collinearly on
/// top of an earlier one (round caps bulge under half the stroke width,
/// well inside this).
const SEGMENT_OBSTACLE_PAD: f32 = FAN_SEP / 2.0;

// ============ M11.5 — one obstacle world, one routing regime ============
//
// Before this milestone the routing regime differed by edge kind, with
// different guarantees each: direct edges orthogonalized blind (no obstacle
// check at all — only forced sides got the ladder), cross-boundary edges
// routed obstacle-aware against node rects only, and forced edges alone
// saw already-routed peer edges as obstacles. The special-case gates
// (around-target, title-detour, forced stubs) were scattered as booleans
// across `assemble` and `cross_boundary_path`, and the per-edge context
// was derived twice. M11.5 restructures this into ONE regime: the same
// [`Obstacles`] world and the same [`route_lca`] ladder for every edge,
// with edge-type knowledge confined to which obstacles and candidate
// routes each edge feeds it (the candidate generators + one dispatch
// point, [`route_edge`]). Everything that worked stays: the compound/LCA
// decomposition, the `(cross, flow)` vocabulary, the ladder, and all
// existing invariants.

/// Half-thickness of the thin band that stands for a subgraph frame's drawn
/// outline: the renderer strokes the frame with 1.5 px centered on the
/// rect's edge line, so the band covers exactly the stroke.
const BORDER_HALF: f32 = 0.75;
/// Clearance a route keeps from a frame's border band unless it touches the
/// outline at a designated crossing. Deliberately smaller than
/// [`ROUTE_PAD`]: routes legitimately work right up against the frames they
/// connect to — a fan lane runs [`LANE_INSET`] (5 px) from the rep frame's
/// outline — while still barring the collinear outline-riding the absorbed
/// M12 sets out to eliminate.
const BORDER_PAD: f32 = 2.0;
/// How far outside a frame's outline the around-target route runs, so it
/// neither rides on the outline nor hugs it inside [`BORDER_PAD`]'s
/// clearance (absorbed M12: edges avoid frame outlines).
const AROUND_CLEAR: f32 = 5.0;

/// The one obstacle world every router routes against (M11.5). One model
/// for every edge kind — direct, cross-boundary, forced, self-loop — and
/// one clearance function pair ([`Obstacles::seg_clear`] /
/// [`Obstacles::clear`]) that [`track_route`]'s internal check also uses,
/// eliminating the class of latent bug M11 hit once (`track_route`
/// clear-checking un-inflated rects while its caller inflated them).
struct Obstacles {
    /// Node interiors (absolute rects). A route keeps [`ROUTE_PAD`] clear
    /// of them — except that it may touch one at a designated port: a
    /// segment end lying on the node's boundary, perpendicular to the edge
    /// it touches, with the rest of the segment staying outside the node
    /// (ports legitimately sit on node boundaries; a pass-through, or a
    /// leg sliding along an edge, is barred).
    nodes: Vec<(f32, f32, f32, f32)>,
    /// Previously routed edges' segments, each pre-inflated by
    /// [`SEGMENT_OBSTACLE_PAD`]. Obstacles for *all* edges now (not only
    /// forced ones), so no two edges lay collinearly on top of each other
    /// regardless of which edge happened to be forced. Checked at the same
    /// [`ROUTE_PAD`] inflation as nodes, with no touch exemption.
    segments: Vec<(f32, f32, f32, f32)>,
    /// Thin bands centered on subgraph frames' drawn outlines (absorbed
    /// M12). A route keeps only [`BORDER_PAD`] clear of a band, and may
    /// touch/cross one only at a designated crossing: a segment end lying
    /// on the outline, perpendicular to it. Riding along an outline
    /// (parallel) is always barred, and so is a transversal crossing
    /// without a recorded waypoint — crossing a frame is intentional and
    /// recorded ([`record_border_crossings`], and the stub builders'
    /// `line_rect_exit` clipping). A frame's interior stays legal: the
    /// bands, not the frame rect, are the obstacle.
    borders: Vec<(f32, f32, f32, f32)>,
}

impl Obstacles {
    fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            segments: Vec::new(),
            borders: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.segments.is_empty() && self.borders.is_empty()
    }

    fn push_node(&mut self, r: (f32, f32, f32, f32)) {
        self.nodes.push(r);
    }

    fn push_frame(&mut self, r: (f32, f32, f32, f32)) {
        self.borders.extend(border_bands(r));
    }

    fn push_segment(&mut self, s0: (f32, f32), s1: (f32, f32)) {
        self.segments.push(segment_rect(s0, s1, SEGMENT_OBSTACLE_PAD));
    }

    /// Is the polyline clear of the whole world?
    fn clear(&self, pts: &[(f32, f32)]) -> bool {
        pts.windows(2).all(|w| self.seg_clear(w[0], w[1]))
    }

    /// Is one segment clear of the whole world? The single clearance rule
    /// every router (and [`track_route`]'s internal check) uses.
    fn seg_clear(&self, p0: (f32, f32), p1: (f32, f32)) -> bool {
        self.nodes.iter().all(|&r| seg_clear_solid(p0, p1, r))
            && self.segments.iter().all(|&r| seg_clear_padded(p0, p1, r))
            && self.borders.iter().all(|&b| seg_clear_border(p0, p1, b))
    }
}

/// Inflate a rect by `d` on every side.
fn inflate(r: (f32, f32, f32, f32), d: f32) -> (f32, f32, f32, f32) {
    (r.0 - d, r.1 - d, r.2 + 2.0 * d, r.3 + 2.0 * d)
}

/// The four thin bands centered on a frame rect's edges — the obstacle
/// standing for the frame's drawn outline. Top/bottom bands span the full
/// width (plus the stroke) so the corners are covered; the side bands fill
/// the middle.
fn border_bands(r: (f32, f32, f32, f32)) -> Vec<(f32, f32, f32, f32)> {
    let (x, y, w, h) = r;
    let t = 2.0 * BORDER_HALF;
    let h2 = BORDER_HALF;
    vec![
        (x - h2, y - h2, w + t, t),
        (x - h2, y + h - h2, w + t, t),
        (x - h2, y, t, h),
        (x + w - h2, y, t, h),
    ]
}

/// Is the axis-aligned segment `p0`→`p1` clear of a *solid* rect `r` (a
/// node interior)? It must keep [`ROUTE_PAD`] clear of `r`, except for a
/// designated port touch: the overlap confined to one segment end whose
/// point lies on `r`'s boundary, arriving/leaving perpendicular to the
/// edge it touches, with the rest of the segment outside the raw rect (no
/// pass-through — a segment ending on the far boundary after crossing the
/// interior is rejected).
fn seg_clear_solid(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    let Some((t0, t1)) = segment_rect_interval(p0, p1, inflate(r, ROUTE_PAD)) else {
        return true;
    };
    for (t, p_touch, p_other, at_start) in [(t0, p0, p1, true), (t1, p1, p0, false)] {
        let at_end = if at_start { t <= 1e-3 } else { t >= 1.0 - 1e-3 };
        if !at_end
            || !on_rect_boundary(p_touch, r)
            || !perp_boundary_touch(p0, p1, p_touch, r)
            || segment_intersects_rect(p_other, p_touch, r)
        {
            continue;
        }
        return true;
    }
    false
}

/// Is the axis-aligned segment clear of an already-routed edge's segment
/// rect (pre-inflated by [`SEGMENT_OBSTACLE_PAD`])? Only *collinear riding*
/// is barred: a candidate parallel to the earlier segment's long axis may
/// not overlap its padded rect (so no two edges lay on top of each other —
/// the M9 invariant — and parallel runs closer than the pad are pushed
/// apart), while a perpendicular crossing is legal — edges cross; the
/// invariant bars laying on top, not crossing. The pad (5 px) stays below
/// [`FAN_SEP`] (10 px) so legitimate parallel fan lanes still clear.
fn seg_clear_padded(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    let Some((_t0, _t1)) = segment_rect_interval(p0, p1, r) else {
        return true;
    };
    // Riding = the candidate is parallel to the earlier segment's long axis
    // (both axis-aligned: same orientation) and overlaps its padded rect.
    let vertical = (p1.0 - p0.0).abs() < 1e-6;
    let rides = vertical == (r.3 > r.2);
    !rides
}

/// Is the axis-aligned segment clear of a frame's border band `b`? It must
/// keep [`BORDER_PAD`] clear of the band, except at a designated crossing:
/// the overlap confined to one segment end whose point lies on the band's
/// centerline (the drawn outline), with the segment perpendicular to the
/// outline. Riding along the outline (parallel) is always barred.
fn seg_clear_border(p0: (f32, f32), p1: (f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let Some((t0, t1)) = segment_rect_interval(p0, p1, inflate(b, BORDER_PAD)) else {
        return true;
    };
    let perpendicular = if b.3 <= b.2 {
        (p1.0 - p0.0).abs() < 1e-6
    } else {
        (p1.1 - p0.1).abs() < 1e-6
    };
    if !perpendicular {
        return false;
    }
    for (t, p_touch, at_start) in [(t0, p0, true), (t1, p1, false)] {
        let at_end = if at_start { t <= 1e-3 } else { t >= 1.0 - 1e-3 };
        if at_end && on_band_centerline(p_touch, b) {
            return true;
        }
    }
    false
}

/// Is `p` on the boundary of rect `r` (within a small epsilon)?
fn on_rect_boundary(p: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    let (x, y, w, h) = r;
    let (px, py) = p;
    let on_v = (px - x).abs() < 1e-2 || (px - (x + w)).abs() < 1e-2;
    let on_h = (py - y).abs() < 1e-2 || (py - (y + h)).abs() < 1e-2;
    let in_x = px >= x - 1e-2 && px <= x + w + 1e-2;
    let in_y = py >= y - 1e-2 && py <= y + h + 1e-2;
    (on_v && in_y) || (on_h && in_x)
}

/// Does the axis-aligned segment `p0`→`p1` touch rect `r`'s boundary at
/// `p` *perpendicular* to the edge it touches (arriving/leaving straight
/// through the boundary, not sliding along it)? A vertical segment may
/// touch a horizontal edge (top/bottom); a horizontal segment a vertical
/// edge (left/right).
fn perp_boundary_touch(
    p0: (f32, f32),
    p1: (f32, f32),
    p: (f32, f32),
    r: (f32, f32, f32, f32),
) -> bool {
    let vertical = (p1.0 - p0.0).abs() < 1e-6;
    let (x, y, _w, h) = r;
    if vertical {
        ((p.1 - y).abs() < 1e-2 || (p.1 - (y + h)).abs() < 1e-2)
            && p.0 >= x - 1e-2
            && p.0 <= x + r.2 + 1e-2
    } else {
        ((p.0 - x).abs() < 1e-2 || (p.0 - (x + r.2)).abs() < 1e-2)
            && p.1 >= y - 1e-2
            && p.1 <= y + h + 1e-2
    }
}

/// Is `p` on the centerline (the drawn outline) of band `b`?
fn on_band_centerline(p: (f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let (x, y, w, h) = b;
    if h <= w {
        (p.1 - (y + h / 2.0)).abs() < 1e-2 && p.0 >= x - 1e-2 && p.0 <= x + w + 1e-2
    } else {
        (p.0 - (x + w / 2.0)).abs() < 1e-2 && p.1 >= y - 1e-2 && p.1 <= y + h + 1e-2
    }
}

/// Insert waypoints where an axis-aligned segment crosses a frame band's
/// centerline transversally mid-segment, so the crossing is a designated
/// waypoint the clearance check exempts (M11.5: crossing a frame is
/// intentional and recorded — the within-frame stub builders already do
/// this via `line_rect_exit` clipping; the around-target route is the one
/// generator that still needs it). Crossings already marked by an endpoint
/// on the centerline are left alone.
fn record_border_crossings(
    pts: &[(f32, f32)],
    bands: &[(f32, f32, f32, f32)],
) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(pts.len() + bands.len());
    for w in pts.windows(2) {
        let (p0, p1) = (w[0], w[1]);
        let mut crossings: Vec<(f32, (f32, f32))> = Vec::new();
        for &b in bands {
            let (t, q) = match band_crossing(p0, p1, b) {
                Some(x) => x,
                None => continue,
            };
            crossings.push((t, q));
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.push(p0);
        for (_, q) in crossings {
            out.push(q);
        }
    }
    out.push(*pts.last().expect("at least two points"));
    dedup_consecutive(&mut out);
    out
}

/// Where does the axis-aligned segment `p0`→`p1` cross band `b`'s
/// centerline transversally mid-segment (neither endpoint on the
/// centerline)? Returns the parameter and the crossing point.
fn band_crossing(
    p0: (f32, f32),
    p1: (f32, f32),
    b: (f32, f32, f32, f32),
) -> Option<(f32, (f32, f32))> {
    let (bx, by, bw, bh) = b;
    if bh <= bw {
        // Horizontal band: centerline y = by + bh/2.
        if (p1.0 - p0.0).abs() > 1e-6 {
            return None; // not vertical, so not a perpendicular crossing
        }
        let cy = by + bh / 2.0;
        let (ya, yb) = (p0.1, p1.1);
        if (ya - cy).abs() < 1e-2 || (yb - cy).abs() < 1e-2 || (ya < cy) == (yb < cy) {
            return None;
        }
        let t = (cy - ya) / (yb - ya);
        let x = p0.0;
        if x >= bx - 1e-2 && x <= bx + bw + 1e-2 {
            Some((t, (x, cy)))
        } else {
            None
        }
    } else {
        if (p1.1 - p0.1).abs() > 1e-6 {
            return None;
        }
        let cx = bx + bw / 2.0;
        let (xa, xb) = (p0.0, p1.0);
        if (xa - cx).abs() < 1e-2 || (xb - cx).abs() < 1e-2 || (xa < cx) == (xb < cx) {
            return None;
        }
        let t = (cx - xa) / (xb - xa);
        let y = p0.1;
        if y >= by - 1e-2 && y <= by + bh + 1e-2 {
            Some((t, (cx, y)))
        } else {
            None
        }
    }
}

/// The cross-axis coordinate of a point relative to a side: `x` for a
/// Top/Bottom side, `y` for a Left/Right side.
fn cross_of(p: (f32, f32), side: Side) -> f32 {
    match side {
        Side::Top | Side::Bottom => p.0,
        Side::Left | Side::Right => p.1,
    }
}

/// Return `p` with its cross-coordinate (the one [`cross_of`] reads) replaced
/// by `c`, keeping the flow-coordinate. Used to move a port point onto its
/// separated cross-coordinate along its own side.
fn set_cross(p: (f32, f32), side: Side, c: f32) -> (f32, f32) {
    match side {
        Side::Top | Side::Bottom => (c, p.1),
        Side::Left | Side::Right => (p.0, c),
    }
}

/// The `(lo, hi)` cross-axis span of a node's `side` (the edge the ports sit
/// on): the x-span for Top/Bottom, the y-span for Left/Right.
fn side_cross_span(node: (f32, f32, f32, f32), side: Side) -> (f32, f32) {
    let (x, y, w, _h) = node;
    match side {
        Side::Top | Side::Bottom => (x, x + w),
        Side::Left | Side::Right => (y, y + node.3),
    }
}

/// The flow-coordinate of a node's `side` (the side's position along the
/// flow axis): y for Top/Bottom, x for Left/Right.
fn side_flow_coord(node: (f32, f32, f32, f32), side: Side) -> f32 {
    let (x, y, _w, h) = node;
    match side {
        Side::Top => y,
        Side::Bottom => y + h,
        Side::Left => x,
        Side::Right => x + node.2,
    }
}

/// The page-space [`Side`] an edge's `from=` / `to=` attribute requests
/// (M11). Sides are page-space in both the attribute and the assembled
/// geometry, so this is the identity mapping.
fn side_of_edge(s: crate::ast::EdgeSide) -> Side {
    match s {
        crate::ast::EdgeSide::Top => Side::Top,
        crate::ast::EdgeSide::Bottom => Side::Bottom,
        crate::ast::EdgeSide::Left => Side::Left,
        crate::ast::EdgeSide::Right => Side::Right,
    }
}

/// Push `p` outward from `side`'s plane by `d` (along the side's outward
/// normal). Used by the M11 forced-side routers: a route's first move from
/// a forced port must leave the node, so routing starts from an escaped
/// point outside the box.
fn escape_point(p: (f32, f32), side: Side, d: f32) -> (f32, f32) {
    match side {
        Side::Top => (p.0, p.1 - d),
        Side::Bottom => (p.0, p.1 + d),
        Side::Left => (p.0 - d, p.1),
        Side::Right => (p.0 + d, p.1),
    }
}

/// The flow-coordinate of the port on `rect`'s `side` at cross-axis
/// coordinate `cross`, along `axis`. Unlike [`side_flow_coord`] — which
/// assumes the side lies along the flow axis (the implicit choice always
/// does) — this works for a forced side perpendicular to the flow too
/// (e.g. `from="right"` on a vertical-flow edge: the flow coordinate is
/// the port's y, not the frame's x). For an aligned side it equals
/// [`side_flow_coord`].
fn port_flow(
    rect: (f32, f32, f32, f32),
    side: Side,
    cross: f32,
    axis: FlowAxis,
) -> f32 {
    cross_flow(port_at_cross(rect, side, cross), axis).1
}

/// Which side of `node` the port point `p` lies on (the nearest edge).
fn side_of_port(node: (f32, f32, f32, f32), p: (f32, f32)) -> Side {
    let (x, y, w, h) = node;
    let (px, py) = p;
    let dl = (px - x).abs();
    let dr = (px - (x + w)).abs();
    let dt = (py - y).abs();
    let db = (py - (y + h)).abs();
    let m = dl.min(dr).min(dt).min(db);
    if (m - dl).abs() < 1e-4 {
        Side::Left
    } else if (m - dr).abs() < 1e-4 {
        Side::Right
    } else if (m - dt).abs() < 1e-4 {
        Side::Top
    } else {
        Side::Bottom
    }
}

/// One cross-boundary edge's LCA geometry gathered for the lane pass.
struct LaneGeom {
    edge: usize,
    rep_from: ItemRef,
    rep_to: ItemRef,
    from_side: Side,
    to_side: Side,
    rep_from_rect: (f32, f32, f32, f32),
    rep_to_rect: (f32, f32, f32, f32),
}

/// A port-separation endpoint: one side of one edge that touches one node.
struct PortEnd {
    edge: usize,
    is_from: bool,
    node_idx: usize,
    node_rect: (f32, f32, f32, f32),
    side: Side,
    /// The other endpoint node's centre cross-coordinate along this side's
    /// cross axis — used to order and place the fanned port.
    other_cross: f32,
}

/// Assign fanned port cross-coordinates to node sides carrying more than one
/// edge. Returns a map from `(edge, is_from)` to the assigned cross; sides
/// with a single edge are absent (the caller keeps the node's centre).
///
/// Each edge's port is placed at its *other* endpoint's cross-coordinate
/// projected onto this side (clamped to the side, inset from the corners), so
/// an edge to a directly-aligned target stays straight; edges sharing a side
/// are then nudged apart to at least [`FAN_SEP`].
fn separate_ports(ends: &[PortEnd]) -> std::collections::HashMap<(usize, bool), f32> {
    use std::collections::HashMap;
    let mut groups: HashMap<(usize, Side), Vec<usize>> = HashMap::new();
    for (k, e) in ends.iter().enumerate() {
        groups.entry((e.node_idx, e.side)).or_default().push(k);
    }
    let mut out: HashMap<(usize, bool), f32> = HashMap::new();
    for ((_node_idx, side), idxs) in &groups {
        if idxs.len() < 2 {
            continue;
        }
        let rect = ends[idxs[0]].node_rect;
        let (lo, hi) = side_cross_span(rect, *side);
        let usable_lo = lo + PORT_INSET;
        let usable_hi = hi - PORT_INSET;
        if usable_hi <= usable_lo + 1e-3 {
            continue;
        }
        // Sort by the other endpoint's cross-coordinate (a stable ordering
        // that keeps fanned edges from crossing each other).
        let mut order: Vec<usize> = idxs.clone();
        order.sort_by(|&a, &b| ends[a].other_cross.total_cmp(&ends[b].other_cross));
        // Desired port = the other endpoint's cross, clamped to the side.
        let mut assigned: Vec<f32> = order
            .iter()
            .map(|&i| ends[i].other_cross.clamp(usable_lo, usable_hi))
            .collect();
        // Enforce minimum fan separation left-to-right, then clamp, then a
        // right-to-left pass to restore separation if clamping bunched an end.
        for i in 1..assigned.len() {
            if assigned[i] - assigned[i - 1] < FAN_SEP {
                assigned[i] = assigned[i - 1] + FAN_SEP;
            }
        }
        for v in assigned.iter_mut() {
            *v = v.clamp(usable_lo, usable_hi);
        }
        for i in (1..assigned.len()).rev() {
            if assigned[i] - assigned[i - 1] < FAN_SEP {
                assigned[i - 1] = assigned[i] - FAN_SEP;
            }
        }
        for v in assigned.iter_mut() {
            *v = v.clamp(usable_lo, usable_hi);
        }
        for (k, &i) in order.iter().enumerate() {
            out.insert((ends[i].edge, ends[i].is_from), assigned[k]);
        }
    }
    out
}

/// One cross-boundary edge's data for lane assignment.
struct LaneEdge {
    edge: usize,
    rep_from: ItemRef,
    from_side: Side,
    rep_to: ItemRef,
    from_flow: f32,
    to_flow: f32,
    /// The (separated) source port's cross-coordinate, for lane ordering.
    from_cross: f32,
    /// The flow axis of the edge's LCA level (obstacle band extents are
    /// read along it).
    axis: FlowAxis,
    /// The source's peer obstacles at the LCA level (everything except the
    /// source rep) — shared across a lane group, whose edges share a source
    /// and gap. Used to keep the lane band clear of obstacles that intrude
    /// into the gap.
    obstacles: Vec<(f32, f32, f32, f32)>,
}

/// Assign distinct jog flow-coordinates (lanes) to parallel cross-boundary LCA
/// segments that share a source, source side, and target representative, so
/// they fan out in their shared gap instead of overlapping. Returns a map from
/// edge index to its jog flow-coordinate. A lone edge in a group gets the gap
/// midpoint (the prior behaviour).
fn assign_lanes(edges: &[LaneEdge]) -> std::collections::HashMap<usize, f32> {
    use std::collections::HashMap;
    // Group edges that share a source, source side, and gap (the flow span
    // between the source's exit and the target's entry) — these are the
    // edges whose horizontal jogs would land in the same band and overlap.
    // (Grouping by target would leave same-gap edges like `api2 -> {db,cache,
    // queue}` in separate single-edge groups, all defaulting to the same
    // midpoint.) Flow-coordinates are rounded for the key so float noise
    // from equal-rank placement still groups.
    let mut groups: HashMap<(ItemRef, Side, i64, i64), Vec<usize>> = HashMap::new();
    for (k, e) in edges.iter().enumerate() {
        let key = (
            e.rep_from,
            e.from_side,
            (e.from_flow * 10.0).round() as i64,
            (e.to_flow * 10.0).round() as i64,
        );
        groups.entry(key).or_default().push(k);
    }
    let mut out: HashMap<usize, f32> = HashMap::new();
    for idxs in groups.values() {
        let mut order: Vec<usize> = idxs.clone();
        order.sort_by(|&a, &b| edges[a].from_cross.total_cmp(&edges[b].from_cross));
        // All edges in a group share the same source and gap, hence the same
        // gap and the same source-side peers (the band obstacles, shared).
        let (f0, t0) = (edges[order[0]].from_flow, edges[order[0]].to_flow);
        let gap_lo = f0.min(t0);
        let gap_hi = f0.max(t0);
        let span = (gap_hi - gap_lo).max(0.0);
        let mut usable_lo = gap_lo + LANE_INSET;
        let mut usable_hi = gap_hi - LANE_INSET;
        let n = order.len();
        // Shrink the lane band away from obstacles that intrude into the gap
        // (a jog at a lane inside an obstacle's flow-extent would cross it).
        // Conservative: treat every source-peer obstacle as blocking the full
        // gap width, so no edge is assigned a blocked lane (and the
        // obstacle-aware router never falls back to a colliding midpoint).
        // The band obstacles are the group's shared source-side peers
        // (targets sit at the gap's far end, outside it). Obstacle extents
        // are read along the group's flow axis (generalized from the
        // vertical-only y-extents, so a left-right LCA level's lanes are
        // banded correctly too).
        for &o in &edges[order[0]].obstacles {
            let (olo, ohi) = flow_range(o, edges[order[0]].axis);
            let blo = olo - ROUTE_PAD - 1.0;
            let bhi = ohi + ROUTE_PAD + 1.0;
            if bhi > gap_lo + 1e-3 && blo < gap_hi - 1e-3 {
                if blo <= gap_lo + LANE_INSET {
                    usable_lo = usable_lo.max(bhi);
                }
                if bhi >= gap_hi - LANE_INSET {
                    usable_hi = usable_hi.min(blo);
                }
            }
        }
        for (k, &i) in order.iter().enumerate() {
            let lane = if n > 1 && usable_hi > usable_lo {
                usable_lo + k as f32 * (usable_hi - usable_lo) / (n - 1) as f32
            } else {
                gap_lo + span / 2.0
            };
            out.insert(edges[i].edge, lane);
        }
    }
    out
}

/// The simple orthogonal "Z" from `a` to `b` with its cross-segment at
/// flow-coordinate `lane`: straight if the two ports share a cross-
/// coordinate, otherwise `a -> (a.cross, lane) -> (b.cross, lane) -> b`.
fn simple_z(a: (f32, f32), b: (f32, f32), axis: FlowAxis, lane: f32) -> Vec<(f32, f32)> {
    let (ca, _fa) = cross_flow(a, axis);
    let (cb, _fb) = cross_flow(b, axis);
    if (ca - cb).abs() < 1e-3 {
        return vec![a, b];
    }
    vec![
        a,
        with_flow(ca, lane, axis),
        with_flow(cb, lane, axis),
        b,
    ]
}

/// A rectilinear shortest path (fewest bends among shortest) from `a` to `b`
/// avoiding the obstacle world, routed along a track graph whose tracks are
/// the *local* obstacles' edges (node rects and frame bands) ±
/// [`TRACK_OFF`] plus the two ports' coordinates. Already-routed peer
/// segments are clearance-checked like everything else but do not seed
/// tracks (they would sprawl the track graph over the whole canvas for
/// every edge); when a segment blocks the only corridors, no route is
/// found and the caller falls back (the bounded ladder). Used when the
/// simple Z is blocked; returns the polyline `a -> ... -> b`, or an empty
/// vec if no route exists (caller falls back to the simple Z).
fn track_route(a: (f32, f32), b: (f32, f32), obs: &Obstacles) -> Vec<(f32, f32)> {
    // Vertical tracks (x) and horizontal tracks (y), seeded from node rects
    // and frame bands.
    let mut vts = vec![a.0, b.0];
    let mut hts = vec![a.1, b.1];
    for &(x, y, w, h) in obs.nodes.iter().chain(obs.borders.iter()) {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
    }
    // Bound the routing region to the ports and obstacles (± pad), so the
    // track graph stays small and routes don't wander far afield.
    let (mut minx, mut maxx) = (a.0.min(b.0), a.0.max(b.0));
    let (mut miny, mut maxy) = (a.1.min(b.1), a.1.max(b.1));
    for &(x, y, w, h) in obs.nodes.iter().chain(obs.borders.iter()) {
        minx = minx.min(x - TRACK_OFF);
        maxx = maxx.max(x + w + TRACK_OFF);
        miny = miny.min(y - TRACK_OFF);
        maxy = maxy.max(y + h + TRACK_OFF);
    }
    vts.sort_by(f32::total_cmp);
    hts.sort_by(f32::total_cmp);
    vts.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
    hts.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
    vts.retain(|&x| x >= minx - 1e-2 && x <= maxx + 1e-2);
    hts.retain(|&y| y >= miny - 1e-2 && y <= maxy + 1e-2);
    let nv = vts.len();
    let nh = hts.len();
    if nv == 0 || nh == 0 {
        return Vec::new();
    }
    let n = nv * nh;
    let ai = vts.iter().position(|&x| (x - a.0).abs() < 1e-2).unwrap();
    let aj = hts.iter().position(|&y| (y - a.1).abs() < 1e-2).unwrap();
    let bi = vts.iter().position(|&x| (x - b.0).abs() < 1e-2).unwrap();
    let bj = hts.iter().position(|&y| (y - b.1).abs() < 1e-2).unwrap();
    let start = ai * nh + aj;
    let goal = bi * nh + bj;

    // Clear check for a segment between two track intersections — the same
    // single rule every router uses ([`Obstacles::seg_clear`], with its
    // per-kind inflations and designated-crossing exemptions), so a route
    // found here is one its caller's own check accepts.
    let clear = |x0: f32, y0: f32, x1: f32, y1: f32| -> bool {
        obs.seg_clear((x0, y0), (x1, y1))
    };
    // Neighbours of node (i,j): the adjacent track intersections reachable by
    // a clear orthogonal segment, with the direction of travel (for bend cost).
    // dir: 0=+x,1=-x,2=+y,3=-y.
    let neighbours = |i: usize, j: usize| -> Vec<(usize, usize, usize, f32)> {
        let mut out = Vec::new();
        let x = vts[i];
        let y = hts[j];
        if j + 1 < nh && clear(x, y, x, hts[j + 1]) {
            out.push((i, j + 1, 2, (hts[j + 1] - y).abs()));
        }
        if j > 0 && clear(x, y, x, hts[j - 1]) {
            out.push((i, j - 1, 3, (y - hts[j - 1]).abs()));
        }
        if i + 1 < nv && clear(x, y, vts[i + 1], y) {
            out.push((i + 1, j, 0, (vts[i + 1] - x).abs()));
        }
        if i > 0 && clear(x, y, vts[i - 1], y) {
            out.push((i - 1, j, 1, (x - vts[i - 1]).abs()));
        }
        out
    };

    // Dijkstra over (node, incoming_dir) states so a bend (direction change)
    // can be penalised. 4 dirs; the start has no incoming dir (use 4 = none).
    // A binary heap with lazy deletion keeps the work proportional to the
    // graph size (the state count grows with the world's obstacle count).
    let n_states = n * 5;
    let mut dist = vec![f32::INFINITY; n_states];
    let mut prev = vec![(usize::MAX, usize::MAX); n_states];
    let mut settled = vec![false; n_states];
    let st = |node: usize, d: usize| node * 5 + d;
    dist[st(start, 4)] = 0.0;
    // Distances are non-negative f32, whose bit patterns order like the
    // values — a cheap total order for the heap.
    let mut heap = std::collections::BinaryHeap::new();
    heap.push(std::cmp::Reverse((0.0_f32.to_bits(), st(start, 4))));
    while let Some(std::cmp::Reverse((d_bits, u))) = heap.pop() {
        if settled[u] {
            continue;
        }
        settled[u] = true;
        if f32::from_bits(d_bits).is_infinite() {
            break;
        }
        let node = u / 5;
        let d_in = u % 5;
        if node == goal {
            break;
        }
        let i = node / nh;
        let j = node % nh;
        for (ni, nj, d_out, len) in neighbours(i, j) {
            let v = ni * nh + nj;
            let cost = len + if d_in != 4 && d_in != d_out { BEND_PENALTY } else { 0.0 };
            let vs = st(v, d_out);
            let nd = dist[u] + cost;
            if nd < dist[vs] - 1e-6 {
                dist[vs] = nd;
                prev[vs] = (u, d_out);
                heap.push(std::cmp::Reverse((nd.to_bits(), vs)));
            }
        }
    }

    // Reconstruct: pick the goal state (any dir) with the smallest distance.
    let mut best_goal: Option<usize> = None;
    let mut best = f32::INFINITY;
    for d in 0..5 {
        let s = st(goal, d);
        if dist[s] < best {
            best = dist[s];
            best_goal = Some(s);
        }
    }
    let mut s = match best_goal {
        Some(s) if dist[s].is_finite() => s,
        _ => return Vec::new(),
    };
    let mut path: Vec<(usize, usize)> = Vec::new();
    while s != usize::MAX {
        let node = s / 5;
        path.push((node / nh, node % nh));
        let (p, _d) = prev[s];
        if p == usize::MAX {
            break;
        }
        s = p;
    }
    path.reverse();
    let mut pts: Vec<(f32, f32)> = path.iter().map(|&(i, j)| (vts[i], hts[j])).collect();
    // Collapse collinear interior points (the path runs through some track
    // intersections without turning).
    collapse_collinear(&mut pts);
    // Ensure the endpoints are exactly a and b (snap away float drift).
    if !pts.is_empty() {
        pts[0] = a;
        *pts.last_mut().unwrap() = b;
    }
    pts
}

/// Drop interior points that lie on the straight line through their neighbours
/// (collinear), leaving only the corners of the polyline.
fn collapse_collinear(pts: &mut Vec<(f32, f32)>) {
    if pts.len() < 3 {
        return;
    }
    let mut kept: Vec<(f32, f32)> = Vec::with_capacity(pts.len());
    kept.push(pts[0]);
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if cross.abs() > 1e-3 {
            kept.push(b);
        }
    }
    kept.push(*pts.last().unwrap());
    *pts = kept;
}

/// Route an orthogonal LCA-level segment from port `a` to port `b` (page
/// space) that avoids the obstacle world `obs` — the same ladder and the
/// same world every edge kind now uses (M11.5). It prefers a simple Z at
/// the assigned jog `lane`, then the midpoint, then — if both cross an
/// obstacle — a rectilinear around-route through the track graph. `axis`
/// orients the flow axis so the cross-segment runs across the page
/// perpendicular to it.
fn route_lca(
    a: (f32, f32),
    b: (f32, f32),
    axis: FlowAxis,
    lane: f32,
    obs: &Obstacles,
) -> Vec<(f32, f32)> {
    if obs.is_empty() {
        return simple_z(a, b, axis, lane);
    }
    let z = simple_z(a, b, axis, lane);
    if obs.clear(&z) {
        return z;
    }
    let mid = jog_flow(cross_flow(a, axis).1, cross_flow(b, axis).1);
    let zm = simple_z(a, b, axis, mid);
    if obs.clear(&zm) {
        return zm;
    }
    let tr = track_route(a, b, obs);
    if !tr.is_empty() && obs.clear(&tr) {
        return tr;
    }
    // Last resort: the simple Z at the lane (may cross, but is shortest).
    z
}

// ---- M11 — forced edge sides (`from=` / `to=`) ----
//
// An edge may request the page-space side of each node it connects to. The
// request is honored **literally**: the port lands on the requested side,
// with no automatic reversion to the algorithm's choice (silently
// second-guessing it would leave the user unable to tell whether it did
// anything). A side that contradicts the approach direction is routed
// around — the result may be ugly, and that is the user's cue to change or
// drop the attribute — but it stays orthogonal, never passes through a node
// interior (the `route_lca` ladder's bounded last resort excepted), and
// ports sharing a forced side are still fanned apart (M9 port separation).

/// Route a direct edge honoring forced sides (M11): each port escapes
/// outward along its side's normal by [`SIDE_ESCAPE`], then the
/// [`route_lca`] ladder (simple Z → mid jog → track route → last-resort Z)
/// finds an orthogonal route between the two escaped points that avoids
/// the obstacle world — which includes both endpoint nodes, so the route
/// cannot cut back through either box. The result keeps the forced ports:
/// it starts at `from_port` and ends at `to_port`, with the approach into
/// each port coming from outside the node.
fn forced_direct_path(
    from_port: (f32, f32),
    from_side: Side,
    to_port: (f32, f32),
    to_side: Side,
    axis: FlowAxis,
    obs: &Obstacles,
) -> Vec<(f32, f32)> {
    let a = escape_point(from_port, from_side, SIDE_ESCAPE);
    let b = escape_point(to_port, to_side, SIDE_ESCAPE);
    let lane = jog_flow(cross_flow(a, axis).1, cross_flow(b, axis).1);
    let mut pts = vec![from_port];
    pts.extend(route_lca(a, b, axis, lane, obs));
    pts.push(to_port);
    dedup_consecutive(&mut pts);
    pts
}

/// A real orthogonal loop route for a self-loop with a forced side (M11):
/// out of the `from` side, around the node, and back into the `to` side,
/// instead of the fixed below-node stub. `from_cross` / `to_cross` are the
/// (fanned) port cross-coordinates on each side. Same sides make a
/// rectangular loop hanging off that side; adjacent sides wrap their
/// shared corner; opposite sides wrap a perpendicular corner — the side
/// with more room to the canvas edge (deterministic). Geometry only: the
/// loop's excursion runs [`LOOP_OUT`] beyond the node's sides and ignores
/// other content (a forced loop is the user's explicit shape request).
fn self_loop_path(
    node: (f32, f32, f32, f32),
    from_side: Side,
    to_side: Side,
    from_cross: f32,
    to_cross: f32,
    canvas_w: f32,
    canvas_h: f32,
) -> Vec<(f32, f32)> {
    let p1 = port_at_cross(node, from_side, from_cross);
    let p2 = port_at_cross(node, to_side, to_cross);
    let (x, y, w, h) = node;
    // The offset line of a side pushed `out` beyond the node: its
    // coordinate value, and whether it constrains x (Left/Right) or y
    // (Top/Bottom).
    let line = |side: Side, out: f32| -> (f32, bool) {
        match side {
            Side::Top => (y - out, false),
            Side::Bottom => (y + h + out, false),
            Side::Left => (x - out, true),
            Side::Right => (x + w + out, true),
        }
    };
    let out = LOOP_OUT;
    let (lf, xf) = line(from_side, out);
    let (lt, xt) = line(to_side, out);
    if from_side == to_side {
        // A rectangular loop hanging off one side.
        return vec![p1, escape_point(p1, from_side, out), escape_point(p2, to_side, out), p2];
    }
    if xf != xt {
        // Adjacent sides: the two offset lines meet at the shared corner.
        let c = if xf { (lf, lt) } else { (lt, lf) };
        return vec![
            p1,
            escape_point(p1, from_side, out),
            c,
            escape_point(p2, to_side, out),
            p2,
        ];
    }
    // Opposite sides: wrap a perpendicular corner — the side with more
    // room to the canvas edge.
    let (k, k_is_x) = if xf {
        // Left/Right pair: the corner line is horizontal (wrap over the
        // top or under the bottom).
        let top_room = y;
        let bot_room = canvas_h - (y + h);
        (if bot_room > top_room { y + h + out } else { y - out }, false)
    } else {
        // Top/Bottom pair: the corner line is vertical (wrap around the
        // left or the right side).
        let left_room = x;
        let right_room = canvas_w - (x + w);
        (if right_room > left_room { x + w + out } else { x - out }, true)
    };
    // The two corners where the perpendicular wrap line meets the from- and
    // to-side offset lines.
    let corner1 = if k_is_x { (k, lf) } else { (lf, k) };
    let corner2 = if k_is_x { (k, lt) } else { (lt, k) };
    vec![
        p1,
        escape_point(p1, from_side, out),
        corner1,
        corner2,
        escape_point(p2, to_side, out),
        p2,
    ]
}

/// The thin axis-aligned rect covered by segment `s0`–`s1`, inflated by
/// `t` on every side. Used to turn already-routed edges into obstacles for
/// later forced-side routes (M11), so two forced excursions in one gap
/// cannot lay collinearly on top of each other (the M9 invariant holds
/// without a global router).
fn segment_rect(
    s0: (f32, f32),
    s1: (f32, f32),
    t: f32,
) -> (f32, f32, f32, f32) {
    (
        s0.0.min(s1.0) - t,
        s0.1.min(s1.1) - t,
        (s1.0 - s0.0).abs() + 2.0 * t,
        (s1.1 - s0.1).abs() + 2.0 * t,
    )
}

/// A within-frame stub avoidance for a forced side (M11): the straight
/// stub — both ports on the node's cross-coordinate of the forced side —
/// may pierce a sibling sitting between the node and the frame border; a
/// track route from the escaped node port around the siblings to the rep
/// frame port fixes the common single-frame case. Returns the straight
/// stub unchanged when it is already clear or no around-route exists (the
/// bounded-ladder convention: the forced port is kept either way).
fn force_stub_around_siblings(
    node_port: (f32, f32),
    rep_port: (f32, f32),
    side: Side,
    siblings: &[(f32, f32, f32, f32)],
    inward: bool,
) -> Vec<(f32, f32)> {
    let (s0, s1) = if inward { (rep_port, node_port) } else { (node_port, rep_port) };
    if !siblings
        .iter()
        .any(|&s| segment_intersects_rect(s0, s1, s))
    {
        return vec![s0, s1];
    }
    // The siblings are the obstacle world for this stub's detour (node
    // rects and nested frames alike, solid — a stub must not cross a
    // sibling of any kind).
    let mut obs = Obstacles::empty();
    for &s in siblings {
        obs.push_node(s);
    }
    let a = escape_point(node_port, side, SIDE_ESCAPE);
    let tr = if inward {
        track_route(rep_port, a, &obs)
    } else {
        track_route(a, rep_port, &obs)
    };
    if !tr.is_empty() && obs.clear(&tr) {
        if inward {
            let mut stub = tr;
            stub.push(node_port);
            return stub;
        }
        let mut stub = vec![node_port];
        stub.extend(tr);
        return stub;
    }
    vec![s0, s1]
}

/// The structured routing of one cross-boundary edge: the within-frame stubs
/// plus the two rep-frame ports for the caller to route through the ladder
/// (M11.5: the around-target-frame special case is its own candidate
/// generator, decided at the single dispatch point, so this returns pieces
/// only).
struct CrossPieces {
    /// `src_stub` runs from the source node port to its rep frame port (raw,
    /// pre-orthogonal); `lca` is the two rep-frame ports to be routed;
    /// `tgt_stub` runs from the target rep frame port to the target node port.
    src_stub: Vec<(f32, f32)>,
    lca: [(f32, f32); 2],
    tgt_stub: Vec<(f32, f32)>,
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
/// port), already axis-aligned, or `None` if no clean route is available —
/// a sibling blocks the perpendicular entry on both sides, or the obstacle
/// world blocks both — in which case the caller falls back to the straight
/// stub. Only the single-target-frame, single-or-zero-source-frame case is
/// handled; deeper nesting falls back too.
#[allow(clippy::too_many_arguments)]
fn try_around_target_route(
    _from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    fb: (f32, f32, f32, f32),
    from_chain: &[usize],
    axis: FlowAxis,
    exit: Side,
    entry: Side,
    siblings: &[(f32, f32, f32, f32)],
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
    from_port: (f32, f32),
    lane: f32,
    obs: &Obstacles,
) -> Option<Vec<(f32, f32)>> {
    let (from_cc, from_ff) = cross_flow(from_port, axis);
    let (fb_clo, fb_chi) = cross_range(fb, axis);
    let (to_clo, to_chi) = cross_range(to, axis);
    let to_fc = {
        let (lo, hi) = flow_range(to, axis);
        (lo + hi) / 2.0
    };
    let fb_cc = (fb_clo + fb_chi) / 2.0;

    // The frames whose borders this route may legitimately transversally
    // cross (recorded as designated waypoints): the target frame, and the
    // source's rep frame when the source sits inside one.
    let mut cross_frames: Vec<(f32, f32, f32, f32)> = vec![fb];
    if let Some(&sf) = from_chain.last() {
        cross_frames.push(frame_rect(sf));
    }
    let cross_bands: Vec<(f32, f32, f32, f32)> =
        cross_frames.iter().flat_map(|&r| border_bands(r)).collect();

    // Build the around-route for a given perpendicular side. `around_hi` is
    // the high-cross side — Right for a vertical flow, Bottom for a horizontal
    // flow. The route is built in `(cross, flow)` space and mapped back to
    // `(x, y)` at the end.
    let build = |around_hi: bool| -> Vec<(f32, f32)> {
        // The side run keeps [`AROUND_CLEAR`] off the frame outline (edges
        // avoid frame outlines — absorbed M12); the border transversals it
        // then crosses to reach the target are recorded as designated
        // waypoints.
        let ac = if around_hi { fb_chi + AROUND_CLEAR } else { fb_clo - AROUND_CLEAR };
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
            let gap_f = lane.clamp(lo, hi);
            cf.push((from_cc, gap_f));
            cf.push((ac, gap_f));
            cf.push((ac, to_fc));
        }
        cf.push((node_ac, to_fc));
        let raw: Vec<(f32, f32)> =
            cf.into_iter().map(|(c, f)| with_flow(c, f, axis)).collect();
        record_border_crossings(&raw, &cross_bands)
    };

    // Prefer the around-side toward the source (shorter, no backtracking);
    // fall back to the far side if the near one is blocked — by a sibling
    // crossing or by the obstacle world (an earlier-routed edge or a peer
    // node already in the corridor).
    let near_hi = from_cc >= fb_cc;
    for around_hi in [near_hi, !near_hi] {
        let route = build(around_hi);
        let blocked = route.windows(2).any(|w| {
            siblings
                .iter()
                .any(|&sib| segment_intersects_rect(w[0], w[1], sib))
        });
        if !blocked && obs.clear(&route) {
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
/// instead enters the frame beside the title (past its right edge, or left of
/// its first glyph when that is nearer — [`title_detour_clear_x`]) and jogs
/// across to the node below the title ([`STUB_JOG_CLEARANCE`] above the
/// node), keeping the title clear. When the node is already clear of the
/// title (the usual case for a narrow title) the stub stays straight; deeper
/// nesting keeps the straight stub (crossing the title no worse than the
/// prior frame-center design). When a title-sized frame leaves no room for
/// either entry, [`layout_level`] grows the frame's width so the entry past
/// the title exists (M10). The returned (possibly diagonal) segments are
/// turned into right-angle bends by [`ortho_chain`] (M7).
#[allow(clippy::too_many_arguments)]
fn cross_boundary_path(
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    from_chain: &[usize],
    to_chain: &[usize],
    lca_dir: Direction,
    from_port: (f32, f32),
    to_port: (f32, f32),
    from_cross: f32,
    to_cross: f32,
    // The edge's exit/entry sides at the LCA level, already overridden by
    // any forced `from=` / `to=` side (M11) by the caller.
    exit: Side,
    entry: Side,
    // Whether each side was forced (M11): forced stubs get a sibling-
    // avoidance ladder. (The around-target-frame gate no longer lives here:
    // it moved to the dispatch point's candidate generator, M11.5.)
    forced_from: bool,
    forced_to: bool,
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
    frame_title_width: &dyn Fn(usize) -> Option<f32>,
    frame_children: &dyn Fn(usize) -> Vec<(f32, f32, f32, f32)>,
) -> CrossPieces {
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
    // Node-aligned rep ports at the (separated) endpoint node's cross-
    // coordinate on its rep frame's facing side. When the chain is empty the
    // endpoint is its own rep, so this is the node's own (separated) port.
    let rep_from_port_aligned = port_at_cross(rep_from, exit, from_cross);
    let rep_to_port_aligned = port_at_cross(rep_to, entry, to_cross);

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

    // Source stub: the endpoint node's port -> its representative's frame
    // port (raw, pre-orthogonal), clipping at any intermediate (nested)
    // source frame boundaries. Empty when the endpoint is its own rep.
    let src_stub = if from_chain.is_empty() {
        Vec::new()
    } else if forced_from && from_chain.len() == 1 && from_detour.is_none() {
        // M11: a forced side is honored literally; if the straight stub
        // would pierce a sibling between the node and the frame border,
        // route around it (single-frame chains only).
        let sibs: Vec<(f32, f32, f32, f32)> = frame_children(from_chain[0])
            .into_iter()
            .filter(|r| !rects_near(*r, from))
            .collect();
        force_stub_around_siblings(from_port, rep_from_port, exit, &sibs, false)
    } else {
        build_stub(
            from_port,
            rep_from_port,
            from_cross,
            from_chain,
            from_detour,
            axis,
            frame_rect,
            /* inward */ false,
        )
    };

    // Target stub: symmetric to the source stub.
    let tgt_stub = if to_chain.is_empty() {
        Vec::new()
    } else if forced_to && to_chain.len() == 1 && to_detour.is_none() {
        let sibs: Vec<(f32, f32, f32, f32)> = frame_children(to_chain[0])
            .into_iter()
            .filter(|r| !rects_near(*r, to))
            .collect();
        force_stub_around_siblings(to_port, rep_to_port, entry, &sibs, true)
    } else {
        build_stub(
            to_port,
            rep_to_port,
            to_cross,
            to_chain,
            to_detour,
            axis,
            frame_rect,
            /* inward */ true,
        )
    };

    CrossPieces {
        src_stub,
        lca: [rep_from_port, rep_to_port],
        tgt_stub,
    }
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
        // Concretely: box = 3·FONT_SIZE = 42; cylinder adds 2·CYL_PAD + 2·CYL_RY.
        assert_eq!(box_h, 42.0);
        assert_eq!(cyl_h, 42.0 + 2.0 * CYL_PAD + 2.0 * CYL_RY);
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
    fn frame_grows_to_fit_a_wider_title() {
        // A long title on a narrow subgraph: the frame must widen so the
        // title text fits, rather than the title spilling out of the frame.
        let (d, l) = lay(
            "diagram top-down\n\
             subgraph \"A Very Long Cluster Title\"\n\
             a\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 1);
        let f = sub_rect(&l, 0);
        let title = d.subgraphs[0].title.as_deref().unwrap();
        let m = text::measure(title, FRAME_TITLE_FONT_SIZE);
        // Title starts at FRAME_TITLE_X and needs the same clearance past
        // its last glyph.
        assert!(
            f.w >= m.width + 2.0 * FRAME_TITLE_X - 1e-2,
            "frame ({}) too narrow for title (need {})",
            f.w,
            m.width + 2.0 * FRAME_TITLE_X
        );
        // The node still sits inside with a real inset.
        let a = node_rect(&l, "a");
        assert!(contains((f.x, f.y, f.w, f.h), (a.x, a.y, a.w, a.h)));
        assert!(a.x > f.x + 1.0 && a.y > f.y + 1.0);
        assert_all_finite(&l);
    }

    #[test]
    fn multiline_label_sizes_the_node_box() {
        // "Kubernetes\nCluster": the box spans the widest line and is tall
        // enough for both lines (first em box + one line height).
        let (_d, l) = lay("diagram top-down\napp \"Kubernetes\\nCluster\"\n");
        let r = node_rect(&l, "app");
        let wide = text::measure("Kubernetes", FONT_SIZE).width;
        let m = text::measure("Kubernetes\nCluster", FONT_SIZE);
        assert!(
            (r.w - (wide + 2.0 * NODE_PAD_X)).abs() < 1e-2,
            "box width should span the widest line: {} vs {}",
            r.w,
            wide + 2.0 * NODE_PAD_X
        );
        assert!(
            (r.h - (m.height + 2.0 * NODE_PAD_Y)).abs() < 1e-2,
            "box height should span both lines: {} vs {}",
            r.h,
            m.height + 2.0 * NODE_PAD_Y
        );
        assert!(r.h > 3.0 * FONT_SIZE, "two lines must be taller than one");
    }

    #[test]
    fn multiline_title_grows_the_frame_top_inset() {
        // A two-line title's frame is taller than the same diagram with a
        // one-line title: the top inset grows by one title line height (the
        // band the stacked lines ride in).
        let (_d1, l1) = lay(
            "diagram top-down\n\
             subgraph \"Cluster\"\n\
             a\n\
             end\n",
        );
        let (_d2, l2) = lay(
            "diagram top-down\n\
             subgraph \"Kubernetes\\nCluster\"\n\
             a\n\
             end\n",
        );
        let f1 = sub_rect(&l1, 0);
        let f2 = sub_rect(&l2, 0);
        // Widths: the grown title is wider than "Cluster" — check both
        // contain their widest line, and the multiline frame is at least as
        // wide as its widest title line plus insets.
        let title2_w = text::measure("Kubernetes", FRAME_TITLE_FONT_SIZE).width;
        assert!(f2.w >= title2_w + 2.0 * FRAME_TITLE_X - 1e-2);
        // Height: exactly one extra title line height.
        let extra = text::line_height(FRAME_TITLE_FONT_SIZE);
        assert!(
            (f2.h - f1.h - extra).abs() < 1e-2,
            "two-line title frame should be one line height taller: {} vs {}",
            f2.h,
            f1.h
        );
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
        // The K8s frame connection point: on K8s's bottom edge, aligned with
        // api2 (within its x-span — the rep port lines up with the node, not
        // the frame center; M9 may fan it off the exact centre when api2 has
        // several outgoing edges).
        let k8s_bot_y = k8s.y + k8s.h;
        let cp_idx = e
            .points
            .iter()
            .position(|&p| (p.1 - k8s_bot_y).abs() < 1e-2)
            .expect("no K8s bottom connection point on api2->db");
        let cp = e.points[cp_idx];
        assert!(
            cp_idx >= 1,
            "connection point should not be the first waypoint"
        );
        assert!(
            cp.0 >= api2.x - 1e-2 && cp.0 <= api2.x + api2.w + 1e-2,
            "connection point {cp:?} not aligned with api2 {api2:?}"
        );
        // Every segment of the stub (api2 port -> ... -> K8s connection point)
        // must miss api1.
        for seg in e.points[..=cp_idx].windows(2) {
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
    fn cross_boundary_multiline_title_grows_top_inset_further() {
        // Same growth check as `cross_boundary_subgraph_grows_around_immediate_child`
        // but with a two-line title: the title band itself grows by one title
        // line height *before* the cross-boundary padding, so the stub's jog
        // still lands below the whole title.
        let one_line = lay(r#"diagram top-down
src
subgraph "Kubernetes Cluster"
a
end
src --> a
"#);
        let two_lines = lay(r#"diagram top-down
src
subgraph "Kubernetes\nCluster"
a
end
src --> a
"#);
        let f1 = sub_rect(&one_line.1, 0);
        let f2 = sub_rect(&two_lines.1, 0);
        let extra = text::line_height(FRAME_TITLE_FONT_SIZE);
        assert!(
            (f2.h - f1.h - extra).abs() < 1e-2,
            "two-line title frame should be one title line height taller: {} vs {}",
            f2.h,
            f1.h
        );
        let _a1 = node_rect(&one_line.1, "a");
        let a2 = node_rect(&two_lines.1, "a");
        assert!(
            (a2.y - f2.y - (FRAME_TITLE_H + extra + CROSS_FRAME_PAD)).abs() < 1e-2,
            "top inset should be the two-line title band plus cross padding"
        );
        // The jog (STUB_JOG_CLEARANCE above the node's top) stays clear of
        // the whole title: layout reserves a title band of FRAME_TITLE_H +
        // one line height per extra line at the frame top, and the jog lands
        // exactly at that band's bottom edge.
        let e = edge_path(&two_lines.1, "src", "a");
        let band_bottom = f2.y + FRAME_TITLE_H + extra;
        // The stub's final horizontal approach (the jog) is the second-to-
        // last point: last is a's top port.
        let jog_y = e.points[e.points.len() - 2].1;
        assert!(
            (jog_y - (a2.y - STUB_JOG_CLEARANCE)).abs() < 1e-2,
            "stub should jog STUB_JOG_CLEARANCE above the node: jog {jog_y} node top {}",
            a2.y
        );
        assert!(
            jog_y >= band_bottom - 1e-2,
            "jog ({jog_y}) must clear the two-line title band (bottom {band_bottom})"
        );
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&two_lines.1);
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

    // ================= M10 — title-detour robustness =================

    #[test]
    fn title_clear_gap_grows_with_title_length() {
        // The right-edge clearance is a fixed bump plus a length-derived
        // component: a viewer's fallback font can render a measured title
        // wider, and the difference grows with the title's length, so a
        // fixed gap would be eaten (short titles touched, long ones
        // crossed).
        let short = title_clear_gap(text::measure("S", FRAME_TITLE_FONT_SIZE).width);
        let long = title_clear_gap(text::measure("A Very Long Subgraph Title Indeed", FRAME_TITLE_FONT_SIZE).width);
        assert!(short > TITLE_CLEAR_GAP, "fixed bump missing: {short}");
        assert!(long > short, "length-derived component missing: {long} vs {short}");
        assert!(
            long - short > 0.05 * (long_width_hint()),
            "per-length component too small: {long} vs {short}"
        );
    }

    /// Width of the long test title, for the proportional-gap assertion.
    fn long_width_hint() -> f32 {
        text::measure("A Very Long Subgraph Title Indeed", FRAME_TITLE_FONT_SIZE).width
    }

    /// A wide-titled single-frame subgraph with one small child `a`, plus a
    /// cross-boundary edge `src --> a`. The frame's width is determined by
    /// the title (the child is too narrow to widen it), and `a` sits under
    /// the title's left half — the shape of the M10 left-side detour.
    fn left_detour_setup() -> (crate::ast::Diagram, Layout) {
        lay(r#"diagram top-down
src
subgraph "A Very Long Subgraph Title Indeed"
a
end
src --> a
"#)
    }

    /// The `x` at which the `src -> a` edge crosses the frame's top edge.
    fn frame_top_entry_x(l: &Layout, s: &SubgraphRect, to: &str) -> f32 {
        let e = edge_path(l, "src", to);
        e.points
            .iter()
            .find(|p| (p.1 - s.y).abs() < 1e-2)
            .unwrap_or_else(|| panic!("no connection point on the frame top: {:?}", e.points))
            .0
    }

    /// The title text rect of a titled frame (top band, starting at
    /// [`FRAME_TITLE_X`], the measured width wide, glyph height tall).
    fn title_rect(s: &SubgraphRect, title: &str) -> (f32, f32, f32, f32) {
        let tw = text::measure(title, FRAME_TITLE_FONT_SIZE).width;
        (
            s.x + FRAME_TITLE_X,
            s.y + 2.0,
            tw,
            FRAME_TITLE_H - 6.0,
        )
    }

    #[test]
    fn title_sized_frame_left_detour_when_node_sits_left_of_title() {
        // No room past the title's right edge (the title determines the
        // frame width, so the right margin is only FRAME_TITLE_X): the
        // detour takes the left-side entry instead of giving up and running
        // the stub straight through the title. The title's left edge is
        // anchored at FRAME_TITLE_X, so the small base gap is safe there —
        // no font fallback can widen it leftward.
        let (_d, l) = left_detour_setup();
        let a = node_rect(&l, "a");
        let s = sub_rect(&l, 0);
        // The node is under the title's left half.
        let tw = text::measure("A Very Long Subgraph Title Indeed", FRAME_TITLE_FONT_SIZE).width;
        let title_mid = s.x + FRAME_TITLE_X + tw / 2.0;
        assert!(
            cx(a) < title_mid,
            "test shape changed: node center {:.2} not under the title's left half (< {:.2})",
            cx(a),
            title_mid
        );
        // The frame is title-sized and NOT grown: the left entry needs no
        // extra width, so the group keeps its default frame geometry.
        let natural = (tw + 2.0 * FRAME_TITLE_X).max(a.w + 2.0 * FRAME_PAD_X);
        assert!((s.w - natural).abs() < 1e-2, "frame grew unnecessarily: {}", s.w);
        // The entry is left of the title, TITLE_CLEAR_GAP past its first
        // glyph.
        let entry_x = frame_top_entry_x(&l, s, "a");
        let want = s.x + FRAME_TITLE_X - TITLE_CLEAR_GAP;
        assert!(
            (entry_x - want).abs() < 1e-2,
            "entry {entry_x:.2} not at the left-side detour point {want:.2}"
        );
        // And no segment of the edge touches the title text.
        let e = edge_path(&l, "src", "a");
        let tr = title_rect(s, "A Very Long Subgraph Title Indeed");
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], tr),
                "edge segment {:?}->{:?} crosses the title rect {tr:?}",
                w[0],
                w[1]
            );
        }
    }

    #[test]
    fn title_sized_frame_grows_width_for_right_half_detour() {
        // A node under the title's RIGHT half of a title-sized frame: before
        // M10 the detour gave up (no clear band past the title — the right
        // margin is only FRAME_TITLE_X) and the straight stub ran through
        // the title. Now the frame grows sideways so the entry past the
        // title exists; growing keeps the children anchored at the left/top
        // insets, so the group's internal arrangement is untouched.
        let (_d, l) = lay(r#"diagram top-down
src
subgraph "A Very Long Subgraph Title Indeed"
a
b
end
src --> b
"#);
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let s = sub_rect(&l, 0);
        let tw = long_width_hint();
        let title_mid = s.x + FRAME_TITLE_X + tw / 2.0;
        assert!(
            cx(b) >= title_mid,
            "test shape changed: node center {:.2} not under the title's right half (>= {:.2})",
            cx(b),
            title_mid
        );
        // The frame grew to fit a clear entry past the title plus side
        // padding (the natural title-sized width is tw + 2·FRAME_TITLE_X).
        let need = FRAME_TITLE_X + tw + title_clear_gap(tw) + FRAME_PAD_X;
        assert!(
            s.w >= need - 1e-2,
            "frame width {:.2} did not grow to fit the detour entry (need {:.2})",
            s.w,
            need
        );
        assert!(s.w > tw + 2.0 * FRAME_TITLE_X, "frame not grown past natural");
        // The entry sits past the title's right edge by the fallback-safe
        // gap — not clamped inside the title as before.
        let entry_x = frame_top_entry_x(&l, s, "b");
        let want = s.x + FRAME_TITLE_X + tw + title_clear_gap(tw);
        assert!(
            (entry_x - want).abs() < 1e-2,
            "entry {entry_x:.2} not past the title's right edge {want:.2}"
        );
        // No segment of the edge touches the title text.
        let e = edge_path(&l, "src", "b");
        let tr = title_rect(s, "A Very Long Subgraph Title Indeed");
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], tr),
                "edge segment {:?}->{:?} crosses the title rect {tr:?}",
                w[0],
                w[1]
            );
        }
        // The M5 guarantee: the children keep their internal arrangement
        // (anchored at the left/top insets) — growth is sideways only.
        assert!((b.x - s.x - (FRAME_PAD_X + a.w + NODE_SEP)).abs() < 1e-2);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }

    #[test]
    fn title_detour_entry_clears_measured_title_by_the_fallback_gap() {
        // The detour entry past the title's right edge is placed
        // `title_clear_gap` beyond the title's *measured* right edge — the
        // gap now includes the fixed + proportional fallback margins, so a
        // viewer rendering the title with a wider fallback face still does
        // not reach the entry.
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let s = sub_rect(&l, 0);
        let title = "Kubernetes Cluster";
        let tw = text::measure(title, FRAME_TITLE_FONT_SIZE).width;
        let e = edge_path(&l, "lb", "api1");
        let entry_x = e
            .points
            .iter()
            .find(|p| (p.1 - s.y).abs() < 1e-2)
            .expect("no connection point on the K8s frame top")
            .0;
        let want = s.x + FRAME_TITLE_X + tw + title_clear_gap(tw);
        assert!(
            (entry_x - want).abs() < 1e-2,
            "entry {entry_x:.2} not a fallback-safe gap past the title's measured edge {want:.2}"
        );
        assert!(
            entry_x - (s.x + FRAME_TITLE_X + tw) > TITLE_CLEAR_GAP + TITLE_FALLBACK_PAD - 1e-2,
            "entry gap lost the fallback margins"
        );
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
        // `a` centers on the b/c *block span* (edges), `d` centers exactly on
        // the b/c *label centers*. These coincide only when b and c have
        // equal widths; with per-label padding each box is `label +
        // 2·NODE_PAD_X` wide, so tiny glyph-advance differences remain.
        // Allow up to the full width mismatch — alignment is still visually
        // exact for equal-width siblings.
        let width_diff = (b.w - c.w).abs();
        assert!(
            (cx(a) - bc_mid).abs() <= width_diff / 4.0 + 1e-2,
            "apex not centered over b/c"
        );
        assert!((cx(d) - bc_mid).abs() < 1e-2, "sink not centered over b/c");
        assert!(
            (cx(a) - cx(d)).abs() <= width_diff / 4.0 + 1e-2,
            "apex and sink not aligned"
        );
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

    // ================= M9 — edge separation & obstacle avoidance =================

    /// Two axis-aligned segments share a collinear, overlapping span (i.e. the
    /// edges would lay on top of each other) — the failure mode M9 eliminates.
    fn segments_overlap(a0: (f32, f32), a1: (f32, f32), b0: (f32, f32), b1: (f32, f32)) -> bool {
        let same_x = (a0.0 - a1.0).abs() < 1e-2 && (b0.0 - b1.0).abs() < 1e-2 && (a0.0 - b0.0).abs() < 1e-2;
        let same_y = (a0.1 - a1.1).abs() < 1e-2 && (b0.1 - b1.1).abs() < 1e-2 && (a0.1 - b0.1).abs() < 1e-2;
        if same_x {
            let (l1, h1) = (a0.1.min(a1.1), a0.1.max(a1.1));
            let (l2, h2) = (b0.1.min(b1.1), b0.1.max(b1.1));
            return h1.min(h2) - l1.max(l2) > 1e-2;
        }
        if same_y {
            let (l1, h1) = (a0.0.min(a1.0), a0.0.max(a1.0));
            let (l2, h2) = (b0.0.min(b1.0), b0.0.max(b1.0));
            return h1.min(h2) - l1.max(l2) > 1e-2;
        }
        false
    }

    /// Assert no two edges share a collinear overlapping segment, and no edge
    /// segment passes through any node rect other than its own endpoints — the
    /// two M9 guarantees — on the given layout.
    fn assert_no_edge_overlaps_or_node_passage(d: &crate::ast::Diagram, l: &Layout) {
        // edge index -> (from_idx, to_idx) by id.
        let id_index: std::collections::HashMap<&str, usize> = d
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        // No edge lays on top of another.
        for i in 0..l.edges.len() {
            for j in (i + 1)..l.edges.len() {
                let ei = &l.edges[i];
                let ej = &l.edges[j];
                for k in 0..ei.points.len() - 1 {
                    for m in 0..ej.points.len() - 1 {
                        assert!(
                            !segments_overlap(ei.points[k], ei.points[k + 1], ej.points[m], ej.points[m + 1]),
                            "edges {}->{} and {}->{} overlap (seg {k} / {m})",
                            ei.from, ei.to, ej.from, ej.to
                        );
                    }
                }
            }
        }
        // No edge passes through a node that is not one of its endpoints.
        for (ei, e) in l.edges.iter().enumerate() {
            let from_idx = id_index[e.from.as_str()];
            let to_idx = id_index[e.to.as_str()];
            let _ = ei;
            for k in 0..e.points.len() - 1 {
                let p0 = e.points[k];
                let p1 = e.points[k + 1];
                for (ni, n) in l.nodes.iter().enumerate() {
                    if ni == from_idx || ni == to_idx {
                        continue;
                    }
                    assert!(
                        !segment_intersects_rect(p0, p1, (n.x, n.y, n.w, n.h)),
                        "edge {}->{} segment {p0:?}->{p1:?} passes through node {}",
                        e.from, e.to, n.id
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_finance_for_inspection() {
        let (d, l) = lay(include_str!("../examples/finance.mmd"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        for n in &nodes {
            println!("  {:<12} x={:7.1} y={:7.1} w={:5.1} h={:5.1}", n.id, n.x, n.y, n.w, n.h);
        }
        for s in &l.subgraphs {
            println!("  subgraph #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}", s.index, d.subgraphs[s.index].title, s.x, s.y, s.w, s.h);
        }
        for e in &l.edges {
            let pts: Vec<String> = e.points.iter().map(|(x, y)| format!("({:.1},{:.1})", x, y)).collect();
            println!("  {:<10} -> {:<10} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn infra_edges_do_not_overlap_or_pass_through_nodes() {
        let (d, l) = lay(include_str!("../examples/infra.mmd"));
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(is_orthogonal(&e.points), "{}->{} not orthogonal", e.from, e.to);
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }

    #[test]
    fn subdirection_edges_do_not_overlap_or_pass_through_nodes() {
        let (d, l) = lay(include_str!("../examples/subdirection.mmd"));
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(is_orthogonal(&e.points), "{}->{} not orthogonal", e.from, e.to);
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }

    #[test]
    fn finance_edges_do_not_overlap_or_pass_through_nodes() {
        // The finance diagram: a User reaching a VPN and fanning out to many
        // cvms — the two M9 stress cases. The thick `User --> pubclb` edge
        // and the cross-boundary `User --> VPN` edge share User's bottom
        // side, and five `VPN --> cvm` edges share VPN's bottom.
        let (d, l) = lay(include_str!("../examples/finance.mmd"));
        assert_all_finite(&l);
        assert_eq!(d.edges.len(), l.edges.len());
        for e in &l.edges {
            assert!(is_orthogonal(&e.points), "{}->{} not orthogonal", e.from, e.to);
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
        // The User->VPN edge must not pass through the pubclb node sitting
        // beside VPN (the M9 stress case: the jog through a peer); it routes
        // around it.
        let prd = node_rect(&l, "pubclb");
        let uv = edge_path(&l, "user", "vpn");
        for w in uv.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], rect_of(prd)),
                "User->VPN passes through pubclb"
            );
        }
    }

    #[test]
    fn cross_boundary_lca_routes_around_a_peer_node() {
        // A cross-boundary edge whose LCA segment would jog through a peer
        // node routes around it instead. Here `src` (top-level) -> `t` (in
        // subgraph S, below) with `mid` (a peer) sitting between them on the
        // same x as both endpoints, so a single midpoint jog cannot clear it.
        let (_d, l) = lay(r#"diagram top-down
src
mid
subgraph "S"
t
end
src --> t
"#);
        let mid = node_rect(&l, "mid");
        let e = edge_path(&l, "src", "t");
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], rect_of(mid)),
                "src->t segment {w:?} passes through mid"
            );
        }
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&l);
    }

    // ================= M11 — forced edge sides (`from=` / `to=`) =================

    /// Is point `p` on `n`'s named page-space side (`"top"`, `"bottom"`,
    /// `"left"`, `"right"`)? (Tests can't name the internal [`Side`] enum,
    /// so sides are passed as strings.)
    fn on_side(n: &NodeRect, p: (f32, f32), side: &str) -> bool {
        let eps = 1e-2;
        let (px, py) = p;
        match side {
            "top" => (py - n.y).abs() < eps,
            "bottom" => (py - (n.y + n.h)).abs() < eps,
            "left" => (px - n.x).abs() < eps,
            "right" => (px - (n.x + n.w)).abs() < eps,
            _ => panic!("bad side name {side}"),
        }
    }

    #[test]
    fn forced_from_side_is_honored_in_every_direction() {
        // Each of the four sides, under each of the four directions: the
        // edge's start lands on the requested (page-space) side of the
        // source — whether it agrees with the implicit choice (no reroute)
        // or contradicts it (routed around).
        for dir in ["top-down", "bottom-up", "left-right", "right-left"] {
            for side in ["top", "bottom", "left", "right"] {
                let src = format!("diagram {dir}\na -- from=\"{side}\" --> b\n");
                let (_d, l) = lay(&src);
                let a = node_rect(&l, "a");
                let e = edge_path(&l, "a", "b");
                assert!(
                    on_side(a, e.points[0], side),
                    "{dir} from=\"{side}\": start {:?} not on a's {side} (a = {a:?})",
                    e.points[0]
                );
                assert!(
                    is_orthogonal(&e.points),
                    "{dir} from=\"{side}\" not orthogonal: {:?}",
                    e.points
                );
            }
        }
    }

    #[test]
    fn forced_to_side_is_honored_in_every_direction() {
        for dir in ["top-down", "bottom-up", "left-right", "right-left"] {
            for side in ["top", "bottom", "left", "right"] {
                let src = format!("diagram {dir}\na -- to=\"{side}\" --> b\n");
                let (_d, l) = lay(&src);
                let b = node_rect(&l, "b");
                let e = edge_path(&l, "a", "b");
                assert!(
                    on_side(b, *e.points.last().unwrap(), side),
                    "{dir} to=\"{side}\": end {:?} not on b's {side} (b = {b:?})",
                    e.points.last().unwrap()
                );
                assert!(
                    is_orthogonal(&e.points),
                    "{dir} to=\"{side}\" not orthogonal: {:?}",
                    e.points
                );
            }
        }
    }

    #[test]
    fn contradictory_forced_side_routes_around_the_node() {
        // a --> b (top-down) forced to leave a from its TOP, against the
        // flow: the route wraps around a instead of falling back to a route
        // that pierces it — the forced side is honored literally.
        let (_d, l) = lay("diagram top-down\na -- from=\"top\" --> b\n");
        let a = node_rect(&l, "a");
        let e = edge_path(&l, "a", "b");
        assert!(on_side(a, e.points[0], "top"), "start not on a's top");
        assert!(is_orthogonal(&e.points));
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], (a.x, a.y, a.w, a.h)),
                "a->b segment {w:?} passes through a (fell back, not routed around)"
            );
        }
    }

    #[test]
    fn forced_sides_fan_edges_sharing_a_forced_side() {
        // Two edges both forced out of a's bottom side: their ports fan
        // apart across that side (M9 separation still applies to the
        // forced side).
        let (_d, l) = lay(
            "diagram top-down\n\
             a\n\
             b \"B\"\n\
             c \"C\"\n\
             a -- from=\"bottom\" --> b\n\
             a -- from=\"bottom\" --> c\n",
        );
        let a = node_rect(&l, "a");
        let e1 = edge_path(&l, "a", "b");
        let e2 = edge_path(&l, "a", "c");
        assert!(on_side(a, e1.points[0], "bottom"));
        assert!(on_side(a, e2.points[0], "bottom"));
        let d = (e1.points[0].0 - e2.points[0].0).abs();
        assert!(d >= FAN_SEP - 1e-2, "forced-side fan separation {d:.2} < FAN_SEP");
    }

    #[test]
    fn forced_side_excursion_grows_the_canvas() {
        // Forcing a side that points off the canvas edge (the page margin
        // is 20, the escape [`SIDE_ESCAPE`] is 24) routes outside the
        // laid-out content bounds; the canvas grows to contain every drawn
        // point — the renderer's viewBox comes straight from width/height.
        let (_d, l_nat) = lay("diagram top-down\na --> b\n");
        let (_d, l) = lay("diagram top-down\na -- from=\"left\" --> b\n");
        assert!(
            l.width > l_nat.width + 1e-2,
            "canvas did not grow for the forced excursion: {} vs {}",
            l.width,
            l_nat.width
        );
        // Everything drawn sits inside the (grown) canvas.
        for e in &l.edges {
            for &(px, py) in &e.points {
                assert!(px >= -1e-3 && px <= l.width + 1e-3, "point x {px} outside canvas");
                assert!(py >= -1e-3 && py <= l.height + 1e-3, "point y {py} outside canvas");
            }
        }
        for n in &l.nodes {
            assert!(n.x >= -1e-3 && n.x + n.w <= l.width + 1e-3);
            assert!(n.y >= -1e-3 && n.y + n.h <= l.height + 1e-3);
        }
    }

    #[test]
    fn forced_self_loop_same_side_makes_a_bump_loop() {
        // Same-side loop: a rectangular bump hanging off the right side,
        // instead of the fixed below-node stub.
        let (_d, l) = lay("diagram top-down\na -- from=\"right\" to=\"right\" --> a\n");
        let a = node_rect(&l, "a");
        let e = edge_path(&l, "a", "a");
        assert!(on_side(a, e.points[0], "right"));
        assert!(on_side(a, *e.points.last().unwrap(), "right"));
        assert!(is_orthogonal(&e.points));
        for &(px, _) in &e.points[1..e.points.len() - 1] {
            assert!(px >= a.x + a.w - 1e-2, "loop interior point at {px} crosses a's body");
        }
    }

    #[test]
    fn forced_self_loop_opposite_sides_wraps_a_corner() {
        // Opposite-side loop: out of the bottom, around one side, back into
        // the top.
        let (_d, l) = lay("diagram top-down\na -- from=\"bottom\" to=\"top\" --> a\n");
        let a = node_rect(&l, "a");
        let e = edge_path(&l, "a", "a");
        assert!(on_side(a, e.points[0], "bottom"));
        assert!(on_side(a, *e.points.last().unwrap(), "top"));
        assert!(is_orthogonal(&e.points));
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], (a.x, a.y, a.w, a.h)),
                "self-loop segment {w:?} passes through a"
            );
        }
    }

    #[test]
    fn forced_side_on_cross_boundary_edge_enters_target_from_that_side() {
        // `src --> t` where t sits inside a subgraph, forced to enter t at
        // its RIGHT side: the LCA segment runs to the frame's right border
        // at t's height, and the within-frame stub enters t's right side.
        let (_d, l) = lay(
            r#"diagram top-down
src
subgraph "S"
t "Target"
end
src -- to="right" --> t
"#,
        );
        let t = node_rect(&l, "t");
        let e = edge_path(&l, "src", "t");
        assert!(on_side(t, *e.points.last().unwrap(), "right"));
        assert!(is_orthogonal(&e.points));
        assert_all_finite(&l);
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_sides_for_inspection() {
        let (_d, l) = lay(include_str!("../examples/sides.mmd"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        for n in &nodes {
            println!("  {:<10} x={:7.1} y={:7.1} w={:5.1} h={:5.1}", n.id, n.x, n.y, n.w, n.h);
        }
        for e in &l.edges {
            let pts: Vec<String> = e.points.iter().map(|(x, y)| format!("({:.1},{:.1})", x, y)).collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn forced_sides_keep_m9_invariants_on_the_example() {
        let (d, l) = lay(include_str!("../examples/sides.mmd"));
        assert_eq!(d.edges.len(), l.edges.len());
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(is_orthogonal(&e.points), "{}->{} not orthogonal", e.from, e.to);
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }
}
