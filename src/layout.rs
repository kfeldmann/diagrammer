//! Layered (Sugiyama-style) layout, extended for **compound graphs** with
//! per-group direction (milestones 2 and 4).
//!
//! The flat engine is the classic four-phase framework, unchanged from M2:
//!
//! 1. **Cycle removal** — a DFS that reverses back-edges so the graph becomes
//!    a DAG (the reversal is internal; the original edge direction is
//!    preserved in the output).
//! 2. **Layering** — longest-path ranking, then component packing: each
//!    weakly connected component (rank-0 singletons included) is re-based
//!    onto a disjoint run of ranks so even edge-less nodes follow the
//!    scope's flow axis instead of crowding into a perpendicular source
//!    band.
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
//! Groups form a containment tree (each node belongs to at most one
//! group; groups nest). Layout is **recursive**: each level (the
//! top-level diagram, or a group) is laid out as a flat graph whose
//! "items" are the level's direct-child real nodes plus its direct-child
//! groups (treated as opaque compound boxes). A group box is sized to
//! fit its own recursively-laid-out contents plus a labeled frame. Each
//! level uses its own effective direction (a group's direction, or the
//! direction inherited from its enclosing level).
//!
//! Each original edge is assigned to the **lowest common ancestor (LCA)**
//! level of its two endpoints: the unique level that contains both endpoints
//! (recursively). At that level the edge is represented between the two
//! endpoints' *representatives* — the direct child of the LCA that is an
//! ancestor-or-self of each endpoint. So an edge between a top-level node and
//! a node deep inside a group becomes, at the top level, an edge between
//! that node and the group's compound box. This keeps compound boxes
//! positioned relative to their connected neighbors without dragging the
//! inner nodes' internal arrangement around — the Mermaid/dagre failure mode
//! this tool exists to escape.
//!
//! ### Edge routing (M20.3 — whole-path maze routing)
//!
//! Every edge is routed by **one whole-path A\* search from source node port
//! to target node port** over a track graph, against **one obstacle model**.
//! Frame-crossing topology is a *search output*: the containment tree's only
//! remaining routing role is the per-edge ancestor/solid classification
//! ([`FrameClass`]) — the frames on the endpoints' ancestor chains are
//! crossable (perpendicular crossings legal, parallel riding barred, a
//! crossing penalty per transversal) and every other frame is fully solid.
//! Group titles are solid obstacles too, so the search routes around them.
//!
//! The old piecewise machinery — the rep model (rep ports, the frame
//! crossing fan, within-frame stubs, title detours, the around-target
//! wrapper) and the M14 coupled/decoupled forced-side duality — is deleted.
//! What survives of the earlier routing work is the shared infrastructure:
//! the track graph ([`track_search`]), the M9 separation pre-passes (port
//! fanning and lane assignment, which the search honors as row
//! *preferences*), and M16's soft peer occupancy + bounded rip-up-and-repair
//! loop (see [`route_edge`] / [`assemble`]).
//!
//! ### The separation pre-passes (M9/M16, re-grounded on search outputs at M20.4)
//!
//! Three deterministic pre-passes run before any search and hand the router
//! *preferences*, never bars:
//!
//! 1. **Port fanning** ([`separate_ports`]) — edges sharing a node side get
//!    distinct port coordinates, ordered by the other endpoint's position.
//! 2. **Lane assignment** ([`assign_lanes`]) — parallel cross-boundary edges
//!    through one gap get distinct jog rows.
//! 3. **Frame-crossing fanning** ([`separate_frame_crossings`], new at M20.4)
//!    — edges entering one *ancestor frame side* get distinct crossing
//!    coordinates. Without it every edge into a frame side picks the side's
//!    cheapest row (all crossings are designated waypoints, so they all cost
//!    the same), the routes collapse onto one column, and the repair loop has
//!    to negotiate them apart — which on a fan into one frame means pushing
//!    one edge all the way around it. The fan is the frame-side analogue of
//!    the port fan, fed into the search as [`Obstacles::crossing_rows`] (seeded
//!    tracks plus a [`CROSSING_DRIFT_COST`] preference). It is measured: on
//!    the `cluster` sample it removes 678.6 px of repair-loop detour.
//!
//! The M16 rip-up-and-repair loop needed no repointing at M20.4: it already
//! reroutes whole paths through [`route_edge`] (the one dispatch point) and
//! derives each edge's reserved port pocket from that edge's *current*
//! polyline ([`pocket_rects`]), so a search output is treated exactly like
//! any other route. What M20.4 changed is that the loop now usually has
//! nothing to do — the pre-passes allocate the shared resources up front
//! (pinned by `repair_loop_has_nothing_to_negotiate_on_the_cluster_sample`).
//!
//! A forced `from=`/`to=` side still binds the endpoint node's port with M11
//! semantics (the port lands on the requested side and escapes outward by
//! [`SIDE_ESCAPE`]). M20.4 settled the routing question the plan left open:
//! grounding forced sides on the search (the plan's "the forced port is
//! simply where the search begins") was tried and rejected — the search's
//! cost model has no term for the outward escape, so it drew shorter but
//! wrong shapes — so a forced *direct* edge keeps the M11 escape-and-ladder
//! and a forced *cross-boundary* edge keeps the escape-and-ladder fallback
//! (the search starts at the forced ports but the ladder is what honors the
//! request). Self-loops keep their dedicated generators. The M5 headline guarantee is unchanged:
//! a group's internal arrangement never moves because an edge crosses its
//! boundary.
//!
//! ## Orthogonal edge routing (M7)
//!
//! Every polyline is axis-aligned. The whole-path search emits orthogonal
//! routes directly (it walks a track graph); the flat engine's waypoints for
//! a direct edge are passed through [`ortho_chain`], which turns each
//! diagonal segment into a right-angle "Z" at the segment's flow midpoint.
//! The arrowhead keeps its `orient="auto"` marker, which the axis-aligned
//! final segment orients along a clean cardinal direction.
//!
//! ## Edge labels (M13)
//!
//! Each labeled edge's anchor is computed here — where the finished geometry
//! (obstacles, lanes, frames, other edges) is known — and stored on its
//! [`EdgePath`] as `label_at`; the renderer just draws the knockout rect +
//! text there. Placement is a deterministic greedy + refinement pass over
//! candidate positions along each polyline (see [`place_labels`]), and the
//! M9 fan/lane separation is label-aware ([`label_pair_sep`]) so parallel
//! knockouts do not cover neighbor lines where the room exists. The knockouts
//! dodge node interiors, rendered arrowheads, other edges' lines, other
//! labels, and group titles — but not group frame borders: a label may sit on
//! (or overhang across) a frame outline. See the M13 section below.

use crate::ast::{Diagram, Direction, Group};
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
/// Page margin around the whole drawing (top level only) — and the minimum
/// clear space the canvas fit in [`assemble`] guarantees between any drawn
/// geometry (node rect, frame rect, edge waypoint, label knockout) and every
/// canvas edge. Strokes and arrowheads paint within a few px of that geometry
/// (round caps ~1 px; an arrowhead's perpendicular spread is at most 5 px at
/// its base), so the visible gap never closes.
const MARGIN: f32 = 20.0;
/// Smallest node size, so even tiny labels get a visible box. Match the
/// per-node sizing: three font heights, wide and tall.
const MIN_NODE_W: f32 = 3.0 * FONT_SIZE;
const MIN_NODE_H: f32 = 3.0 * FONT_SIZE;
/// Edge-label font size (M13). Public so the SVG renderer sizes its edge-label
/// `<text>` to match the knockout rects this module places (see [`label_box`]).
pub const EDGE_LABEL_SIZE: f32 = 12.0;
/// Padding inside an edge label's white knockout rect, each side (M13).
/// Public so the renderer draws the very knockout rect the placement engine
/// scored (see [`label_box`]); keep the two formulas together.
pub const LABEL_PAD: f32 = 3.0;
/// Clearance a label's knockout keeps from a neighboring edge line where the
/// fan/lane spacing can provide it (M13): the gap between two adjacent
/// parallel runs grows to the wider label's half-extent across the runs plus
/// this (see [`label_pair_sep`]).
const LABEL_CLEAR: f32 = 2.5;
/// Cap on the label-aware fan bump (M13). Uncapped, one wide multi-line label
/// would fan its neighbors past the corners of a real node side; the cap keeps
/// a fan usable and leaves the residual crowding to the placement engine's
/// offset candidates. Lanes need no cap — their gap band clamps them.
const FAN_LABEL_CAP: f32 = 24.0;
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

// Frame (group) sizing, in pixels. The frame is a labeled border drawn
// around a group's contents; these define the inset between the frame
// edge and the inner content (the recursive layout's bounding box).
const FRAME_PAD_X: f32 = 1.5 * FONT_SIZE;
const FRAME_PAD_Y: f32 = 8.0;
/// Height reserved at the top of a frame for the title, when present (one
/// title line; multi-line titles grow the inset — see [`frame_insets`]).
const FRAME_TITLE_H: f32 = 20.0;
/// Extra top inset below a titled frame's title text (M20.3): one route's
/// clearance, so a whole-path run can pass between the title and the frame's
/// first rank. This is the band the deleted title detour hand-placed its jog
/// in; the search routes around the title only if the strip exists. Sized
/// above [`ROUTE_PAD`] plus a track gap so a track row fits between the title
/// obstacle and the first node.
const TITLE_BAND_PAD: f32 = 14.0;
/// Inset of the title text from the frame's top-left corner, and the font
/// size it is rendered at. Layout reads these to build the title's routing
/// obstacle ([`title_obstacle_rect`]) — the solid rect the whole-path search
/// keeps clear (M20.3) — and the label placement engine keeps knockouts off
/// the same box. Keep in sync with the renderer's `FRAME_TITLE_X` /
/// `FRAME_TITLE_SIZE` (both in `render/svg.rs`): the title text occupies
/// roughly `x ∈ [frame.x + FRAME_TITLE_X, frame.x + FRAME_TITLE_X +
/// title_width]` at the top of the frame.
const FRAME_TITLE_X: f32 = 10.0;
const FRAME_TITLE_FONT_SIZE: f32 = 12.0;
/// Top offset of the rendered title text inside the frame's title band (M13:
/// the label placement engine keeps knockouts off the title rect). Keep in
/// sync with the renderer's `FRAME_TITLE_Y` (in `render/svg.rs`, 4 px): the
/// title text occupies `y ∈ [frame.y + FRAME_TITLE_TOP, … + lines *
/// line_height]`.
const FRAME_TITLE_TOP: f32 = 4.0;
/// Base clearance a route keeps past a group title's edge (see
/// [`title_obstacle_rect`]). Applied to the title's **left** edge — which is
/// anchored at [`FRAME_TITLE_X`] by the renderer and so cannot be widened by
/// a viewer's font fallback — and, plus the fallback margins
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

/// The clearance a title-detour entry keeps past a title's *right* edge: the
/// base gap ([`TITLE_CLEAR_GAP`]) plus a fixed bump and a length-derived
/// component so a viewer's wider fallback font cannot eat the gap (see
/// [`TITLE_FALLBACK_PAD`] / [`TITLE_FALLBACK_PER_W`]).
fn title_clear_gap(title_width: f32) -> f32 {
    TITLE_CLEAR_GAP + TITLE_FALLBACK_PAD + title_width * TITLE_FALLBACK_PER_W
}

// ---- Public output types (consumed by milestone 3 rendering) ----

/// A laid-out diagram: node rectangles, edge polylines, group frames, and
/// the canvas size. Nodes and edges correspond by index to the resolved
/// [`Diagram`] (declaration order); groups correspond by index to
/// `diagram.groups`.
#[derive(Debug, Clone)]
pub struct Layout {
    pub nodes: Vec<NodeRect>,
    pub edges: Vec<EdgePath>,
    pub groups: Vec<GroupRect>,
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
/// `label_at` is the anchor of the edge's label (M13): the center of the
/// knockout rect + text block the renderer draws there, computed by the label
/// placement engine ([`place_labels`]) against the finished geometry. `None`
/// for an unlabeled edge. A per-edge field, so the index-correspondence
/// invariant is safe.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgePath {
    pub from: String,
    pub to: String,
    pub points: Vec<(f32, f32)>,
    pub label_at: Option<(f32, f32)>,
}

/// A group's frame rectangle, in absolute (page) coordinates. The
/// frame's style (title, border color/fill/line, title text color) lives on
/// the resolved [`crate::ast::Group`]; the renderer zips `diagram.groups`
/// with `layout.groups` (they correspond by index) so this struct only
/// carries geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRect {
    /// Index into `Diagram::groups`.
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
/// group (compound box). Carries the *global* index into `Diagram::nodes`
/// or `Diagram::groups`.
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
/// system. For the top level, "local" is page coordinates. For a group,
/// "local" is the group's inner content space (origin at the frame's
/// content top-left); the parent translates it into place.
struct LevelOut {
    /// `(global node index, x, y, w, h)` for every descendant real node.
    nodes: Vec<(usize, f32, f32, f32, f32)>,
    /// `(global group index, x, y, w, h)` for every descendant frame.
    frames: Vec<(usize, f32, f32, f32, f32)>,
    /// `(global edge index, waypoints)` for direct internal edges (rendered
    /// via flat waypoints). Cross-boundary edges are filled in later.
    edges: Vec<(usize, Vec<(f32, f32)>)>,
    /// Content size of this level (the flat canvas, including its margin).
    w: f32,
    h: f32,
}

/// A recursively-laid-out child group, pending placement into its parent.
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

/// Lay out a resolved [`Diagram`], honoring group containment and
/// per-group direction.
pub fn layout(diagram: &Diagram) -> Layout {
    layout_impl(diagram, false, None)
}

/// Test hook: stop after the M16 routing pass's phase 1, so tests can
/// compare the simultaneous routes (which do not depend on the edges'
/// declaration order) rather than the post-repair final ones (which do, by
/// design — phase 2 negotiates in index order).
#[cfg(test)]
fn layout_phase1(diagram: &Diagram) -> Layout {
    layout_impl(diagram, true, None)
}

/// Test hook: the full pipeline with a trace callback into the phase-2
/// negotiation (which edges get ripped up / frozen each pass).
#[cfg(test)]
fn layout_trace(diagram: &Diagram, trace: &dyn Fn(&str)) -> Layout {
    layout_impl(diagram, false, Some(trace))
}

fn layout_impl(diagram: &Diagram, phase1_only: bool, trace: Option<&dyn Fn(&str)>) -> Layout {
    if diagram.nodes.is_empty() {
        // Still produce frames for any (empty) groups, sized to their
        // padding, so they render as small labeled boxes rather than vanish.
        let top = layout_level(diagram, None, diagram.direction, &[]);
        return assemble(diagram, top, &[], phase1_only, trace);
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
            let lca = lca_level(
                diagram.nodes[fi].group,
                diagram.nodes[ti].group,
                &diagram.groups,
            );
            let rep_from = rep_of(fi, lca, diagram);
            let rep_to = rep_of(ti, lca, diagram);
            let is_direct =
                matches!(rep_from, ItemRef::Node(_)) && matches!(rep_to, ItemRef::Node(_));
            EdgeInfo {
                lca,
                rep_from,
                rep_to,
                is_direct,
            }
        })
        .collect();

    let top = layout_level(diagram, None, diagram.direction, &edge_infos);
    assemble(diagram, top, &edge_infos, phase1_only, trace)
}

/// The absolute geometry `assemble` works from, shared by the context
/// builder and the routing phase.
struct Geometry<'a> {
    diagram: &'a Diagram,
    id_index: std::collections::HashMap<&'a str, usize>,
    node_rect: std::collections::HashMap<usize, (f32, f32, f32, f32)>,
    frame_rect: std::collections::HashMap<usize, (f32, f32, f32, f32)>,
    direct_pts: std::collections::HashMap<usize, Vec<(f32, f32)>>,
    /// Per-level items (nodes + direct-child frames), in absolute
    /// coordinates — the lane-assignment obstacle field (M9): a lane band
    /// shrinks away from the level's other items.
    level_items: std::collections::HashMap<Option<usize>, Vec<LevelItem>>,
}

impl<'a> Geometry<'a> {
    fn node(&self, gi: usize) -> (f32, f32, f32, f32) {
        self.node_rect
            .get(&gi)
            .copied()
            .unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    fn frame(&self, s: usize) -> (f32, f32, f32, f32) {
        self.frame_rect
            .get(&s)
            .copied()
            .unwrap_or((0.0, 0.0, 0.0, 0.0))
    }

    /// The title's routing obstacle rect (M20.3, see
    /// [`title_obstacle_rect`]) — what [`edge_world`] pushes so the
    /// whole-path search routes around the title.
    fn title_obstacle_rect(&self, s: usize) -> Option<(f32, f32, f32, f32)> {
        self.diagram
            .groups
            .get(s)
            .and_then(|sg| sg.title.as_ref())
            .map(|t| title_obstacle_rect(title_text_rect(self.frame(s), t), t))
    }
}

/// One edge's routing context, derived exactly once (M11.5): endpoint rects,
/// containment chains, representatives, effective sides (the implicit choice
/// vs. the forced override resolved in one place), the flat waypoints for a
/// direct edge, the special-case gates, and — after the separation pre-
/// passes — the (possibly fanned) ports and the assigned jog lane. Before
/// this hoist, `assemble` derived all of this twice (pre-pass + routing
/// loop) and kept the two copies in agreement by hand.
#[derive(Clone)]
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
    flow: FlowAxis,
    /// Forced page-space sides (`from=` / `to=`); `None` keeps the implicit
    /// choice (M11). The request applies to the endpoint **node** only
    /// (M14): the group frames the edge crosses keep their own implicit
    /// sides, so a forced side never drags a frame crossing with it.
    forced_from: Option<Side>,
    forced_to: Option<Side>,
    /// The implicit side choice (what the flat engine / [`sides_along`] pick)
    /// — geometrically the side facing the other representative.
    implicit_from: Side,
    implicit_to: Side,
    /// The effective node sides (forced override applied — literal
    /// semantics): where the endpoint nodes' ports sit.
    from_side: Side,
    to_side: Side,
    /// The endpoints' representatives at the LCA level. The rep *ports* are
    /// gone (M20.3 — the search routes port to port), but the
    /// representatives still identify an edge's lane group (which LCA-level
    /// gap it crosses).
    rep_from: ItemRef,
    rep_to: ItemRef,
    /// The flat engine's waypoints (direct edges only; empty otherwise).
    raw: Vec<(f32, f32)>,
    /// Filled after the separation pre-passes: the endpoint nodes'
    /// (possibly fanned) ports and their cross-coordinates, plus the
    /// cross-boundary edge's assigned jog lane in its LCA gap (the M9 lane
    /// pre-pass's separation guarantee, which the search's cross-run
    /// prefers — see [`Obstacles::lane_row`]).
    from_port: (f32, f32),
    to_port: (f32, f32),
    from_cross: f32,
    to_cross: f32,
    lane: f32,
    /// M20.4: the assigned crossing coordinates of this edge's ancestor frame
    /// sides (see [`separate_frame_crossings`]) — `(frame, side, cross)`,
    /// seeded into the whole-path search's tracks and preferred by
    /// [`CROSSING_DRIFT_COST`].
    crossing_rows: Vec<(usize, Side, f32)>,
    /// The per-edge frame world (M20.1): solid/ancestor classification and
    /// its band/solid obstacle fields, built once in [`assemble`]. `None`
    /// only in hand-built test contexts that construct a context directly
    /// (the helpers default it via [`EdgeCtx::frame_world`] access below);
    /// production contexts are always filled in.
    frame_world: Option<EdgeFrameWorld>,
}

impl EdgeCtx {
    /// The edge's frame world (M20.1): the classification built in
    /// [`assemble`], or — for contexts assembled without it (direct
    /// unit-fixture construction) — derived on the spot from the endpoint
    /// nodes' group memberships. Pure; the derived result equals the
    /// precomputed one.
    fn frame_world(&self, geom: &Geometry) -> EdgeFrameWorld {
        if let Some(fw) = &self.frame_world {
            return fw.clone();
        }
        let fg = geom.diagram.nodes[self.fi].group;
        let tg = geom.diagram.nodes[self.ti].group;
        EdgeFrameWorld::build(geom, fg, tg)
    }
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
    let (raw, from_chain, to_chain, implicit_from, implicit_to) = if is_direct {
        let raw = geom.direct_pts.get(&ei).cloned().unwrap_or_default();
        let fs = raw
            .first()
            .map(|&p| side_of_port(from, p))
            .unwrap_or(Side::Bottom);
        let ts = raw
            .last()
            .map(|&p| side_of_port(to, p))
            .unwrap_or(Side::Top);
        (raw, Vec::new(), Vec::new(), fs, ts)
    } else {
        let from_chain = chain_to_lca(diagram.nodes[fi].group, lca, &diagram.groups);
        let to_chain = chain_to_lca(diagram.nodes[ti].group, lca, &diagram.groups);
        let rf = from_chain.last().map(|&s| geom.frame(s)).unwrap_or(from);
        let rt = to_chain.last().map(|&s| geom.frame(s)).unwrap_or(to);
        let (exit, entry) = sides_along(lca_dir, center(rf), center(rt));
        (Vec::new(), from_chain, to_chain, exit, entry)
    };
    let from_side = forced_from.unwrap_or(implicit_from);
    let to_side = forced_to.unwrap_or(implicit_to);
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
        flow: FlowAxis::from_direction(lca_dir),
        forced_from,
        forced_to,
        implicit_from,
        implicit_to,
        from_side,
        to_side,
        rep_from: info.map(|i| i.rep_from).unwrap_or(ItemRef::Node(fi)),
        rep_to: info.map(|i| i.rep_to).unwrap_or(ItemRef::Node(ti)),
        raw,
        from_port: (0.0, 0.0),
        to_port: (0.0, 0.0),
        from_cross: 0.0,
        to_cross: 0.0,
        lane: 0.0,
        crossing_rows: Vec::new(),
        frame_world: None,
    }
}

/// Turn a top-level [`LevelOut`] (page coordinates) into the public
/// [`Layout`]. The routing regime is the global pass (M16): every edge's
/// context is built once, the M9 separation pre-passes run over the
/// contexts, and then every edge — direct, cross-boundary, forced,
/// self-loop — routes through the one dispatch point ([`route_edge`]): a
/// simultaneous pass against the shared world (nodes, borders, reserved
/// port corridors, no peer segments), then a bounded rip-up-and-repair
/// loop against the negotiated world (surviving peers' segments soft).
fn assemble(
    diagram: &Diagram,
    top: LevelOut,
    edge_infos: &[EdgeInfo],
    phase1_only: bool,
    trace: Option<&dyn Fn(&str)>,
) -> Layout {
    use std::collections::{HashMap, HashSet};

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

    // Absolute frame rect per group index, for cross-boundary routing.
    let frame_rect: HashMap<usize, (f32, f32, f32, f32)> = top
        .frames
        .iter()
        .map(|(idx, x, y, w, h)| (*idx, (*x, *y, *w, *h)))
        .collect();

    let mut direct_pts: HashMap<usize, Vec<(f32, f32)>> = HashMap::new();
    for (ei, pts) in &top.edges {
        direct_pts.insert(*ei, pts.clone());
    }

    let mut level_items: HashMap<Option<usize>, Vec<LevelItem>> = HashMap::new();
    for (gi, nd) in diagram.nodes.iter().enumerate() {
        let r = node_rect.get(&gi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
        level_items
            .entry(nd.group)
            .or_default()
            .push((ItemRef::Node(gi), r));
    }
    for (si, sg) in diagram.groups.iter().enumerate() {
        let r = frame_rect.get(&si).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
        level_items
            .entry(sg.parent)
            .or_default()
            .push((ItemRef::Sub(si), r));
    }

    let geom = Geometry {
        diagram,
        id_index,
        node_rect,
        frame_rect,
        direct_pts,
        level_items,
    };

    // ---- M11.5: build each edge's route context once ----------------------
    let mut ctxs: Vec<EdgeCtx> = (0..diagram.edges.len())
        .map(|ei| edge_context(&geom, edge_infos, ei))
        .collect();

    // ---- M20.1: per-edge frame classification ----------------------------
    // One classifier pass over the containment tree: every edge learns its
    // ancestor frames (crossable borders) and solid frames (fully blocked)
    // once, precomputed for the whole-path search regime (M20.2). No
    // routing behavior changes here — today's consumers still read the
    // LCA-level field via `edge_world`, which the classification reproduces
    // exactly for the ancestor side.
    for ctx in ctxs.iter_mut() {
        let fg = diagram.nodes[ctx.fi].group;
        let tg = diagram.nodes[ctx.ti].group;
        ctx.frame_world = Some(EdgeFrameWorld::build(&geom, fg, tg));
    }

    // ---- M13: edge-label knockout sizes ----------------------------------
    // Shared by the spacing pre-passes below (which grow fan/lane gaps with
    // them) and the label placement engine after routing.
    let label_sizes: Vec<Option<(f32, f32)>> = diagram
        .edges
        .iter()
        .map(|e| e.label.as_deref().map(label_box))
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
            frame: false,
            node_rect: ctx.from_rect,
            side: ctx.from_side,
            other_cross: cross_of(center(ctx.to_rect), ctx.from_side),
            cover: side_label_cover(ctx.from_side, label_sizes[ctx.ei]),
            from_id: ctx.from_id.clone(),
            to_id: ctx.to_id.clone(),
        });
        // An around-target-frame edge enters its target from a perpendicular
        // side, so its own side node port is unused — but it is still
        // assigned here (its detoured-fallback path needs a port of its
        // own): the around-router reads the node geometry directly, so
        // fanning this port neither moves the around-entry nor suppresses
        // the around-route.
        port_ends.push(PortEnd {
            edge: ctx.ei,
            is_from: false,
            node_idx: ctx.ti,
            frame: false,
            node_rect: ctx.to_rect,
            side: ctx.to_side,
            other_cross: cross_of(center(ctx.from_rect), ctx.to_side),
            cover: side_label_cover(ctx.to_side, label_sizes[ctx.ei]),
            from_id: ctx.from_id.clone(),
            to_id: ctx.to_id.clone(),
        });
    }
    let sep = separate_ports(&port_ends);

    // Node sides carrying more than one edge end — those ports are the
    // fan's to place, so the sealed-port fallback ([`port_fallback_route`])
    // must not move them. Same grouping rule [`separate_ports`] fans by.
    let mut side_counts: HashMap<(usize, Side), u32> = HashMap::new();
    for pe in &port_ends {
        *side_counts.entry((pe.node_idx, pe.side)).or_default() += 1;
    }
    let shared_sides: HashSet<(usize, Side)> = side_counts
        .into_iter()
        .filter(|&(_, n)| n >= 2)
        .map(|(k, _)| k)
        .collect();

    // ---- Fill the per-edge (possibly fanned) ports from the pre-pass ------
    // The ports come first (M14): the rep crossings — and with them the lane
    // geometry — resolve from them.
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
    }

    // ---- M9 pre-pass 2: lane separation ----------------------------------
    let mut lane_edges: Vec<LaneEdge> = Vec::new();
    for ctx in &ctxs {
        if ctx.is_direct {
            continue;
        }
        // The cross-interval the cross-runs span: the two node ports'
        // separated crosses (M20.3 — the rep ports are gone; the search
        // crosses each frame at the node's own cross, which is what the
        // rep-port convention encoded geometrically).
        let from_cross = ctx.from_cross;
        let to_cross = ctx.to_cross;
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
            // Flow coordinates of the two node ports (the edge's real ends).
            from_flow: cross_flow(ctx.from_port, ctx.flow).1,
            to_flow: cross_flow(ctx.to_port, ctx.flow).1,
            from_cross,
            to_cross,
            axis: ctx.flow,
            cover: lane_label_cover(ctx.flow, label_sizes[ctx.ei]),
            obstacles,
            from_id: ctx.from_id.clone(),
            to_id: ctx.to_id.clone(),
        });
    }
    let lanes = assign_lanes(&lane_edges);

    // Fill the per-edge jog lanes from the pre-pass results.
    for ctx in ctxs.iter_mut() {
        if !ctx.is_direct {
            ctx.lane = lanes.get(&ctx.ei).copied().unwrap_or_else(|| {
                jog_flow(
                    cross_flow(ctx.from_port, ctx.flow).1,
                    cross_flow(ctx.to_port, ctx.flow).1,
                )
            });
        }
    }

    // ---- M20.4 pre-pass: frame-side crossing separation -------------------
    // The whole-path search chooses where each edge crosses its ancestor
    // frames' border bands, and every such crossing is a designated waypoint
    // (free of the crossing penalty) — so two edges entering one frame side
    // both take the side's cheapest row and land on one coordinate. The
    // repair loop then has to negotiate them apart, which on a fan into one
    // frame means pushing one edge all the way around it. Allocate each
    // frame side's crossing coordinates up front (the [`separate_ports`]
    // idea, on the frames the search crosses) and hand each edge its own
    // coordinate to prefer.
    let mut frame_crosses: Vec<FrameCross> = Vec::new();
    for ctx in &ctxs {
        if ctx.is_self_loop {
            continue;
        }
        let fw = ctx.frame_world(&geom);
        // One crossing per ancestor frame the edge's port-to-port span
        // actually straddles. A frame that contains *both* ports is crossed
        // twice (in and out) and its crossings are the search's business, not
        // a fan's; a frame containing neither is not on the span at all.
        for &g in &fw.class.ancestors {
            let f = geom.frame(g);
            if rect_contains_point(f, ctx.from_port) && rect_contains_point(f, ctx.to_port) {
                continue;
            }
            let Some(side) = frame_entry_side(f, ctx.from_port, ctx.to_port) else {
                continue;
            };
            // The crossing coordinate the edge would like: the *inside*
            // endpoint's cross projected onto the side. That is the natural
            // descent — an edge whose target sits under a frame's top border
            // enters above the target — so a lone edge keeps the shape it
            // would draw anyway and a fan only separates the ones that
            // genuinely collide. When neither endpoint is inside the frame
            // (a pass-by between two sibling frames) the nearer endpoint's
            // cross is the closest thing to a natural column.
            let from_in = rect_contains_point(f, ctx.from_port);
            let to_in = rect_contains_point(f, ctx.to_port);
            let want = if to_in {
                cross_of(ctx.to_port, side)
            } else if from_in {
                cross_of(ctx.from_port, side)
            } else if dist_to_rect(ctx.to_port, f) <= dist_to_rect(ctx.from_port, f) {
                cross_of(ctx.to_port, side)
            } else {
                cross_of(ctx.from_port, side)
            };
            // Kept *unclamped*: the fan sorts by the desired coordinate and
            // clamps when it assigns, so several sources fanning into one
            // frame from outside its side span keep their relative order
            // instead of collapsing onto the span's near corner (which is
            // what happens if the want is clamped here).
            frame_crosses.push(FrameCross {
                edge: ctx.ei,
                frame: g,
                side,
                want,
                cover: side_label_cover(side, label_sizes[ctx.ei]),
                from_id: ctx.from_id.clone(),
                to_id: ctx.to_id.clone(),
            });
        }
    }
    let crossing_assign = separate_frame_crossings(&frame_crosses, &geom.frame_rect);
    // Hand each edge the crossing coordinates assigned to it, grouped by the
    // axis the crossing constrains (see [`Obstacles::row_drift_cost`]).
    for ctx in ctxs.iter_mut() {
        ctx.crossing_rows = crossing_assign
            .iter()
            .filter(|&(&(e, _, _), _)| e == ctx.ei)
            .map(|(&(_, g, side), &c)| (g, side, c))
            .collect();
        // Deterministic order (the map's iteration order is not): sort by
        // frame index, then side, then coordinate.
        ctx.crossing_rows.sort_by(|a, b| {
            (a.0, a.1 as u8, a.2.to_bits()).cmp(&(b.0, b.1 as u8, b.2.to_bits()))
        });
    }

    // ---- Routing phase: the global routing pass (M16) ---------------------
    // Phase 1 (simultaneous): every edge routes against the same world —
    // nodes and its LCA-level frame borders, with no peer occupancy at
    // all — so each edge takes its unconstrained-best route, and the
    // result does not depend on declaration order. Phase 2 (repair): a
    // bounded loop scans the routed set for hard violations — a segment
    // through a node or riding a frame border, or two peers riding
    // collinearly — rips up the edges the pockets attribute the conflicts
    // to (a port's reserved pocket has right-of-way: the invader yields),
    // and reroutes them against a world where the surviving peers'
    // segments are soft (a routing cost, not a barrier). Edges that stay
    // violating are frozen: their violation is the bounded best effort,
    // like the ladder's kept-unconditional terminal.
    //
    // One edge's full route attempt against `world` — the dispatch point
    // ([`route_edge`]) plus the sealed-port fallback.
    let attempt = |ctx: &EdgeCtx, world: &Obstacles| -> Vec<(f32, f32)> {
        let mut pts = route_edge(ctx, world, &geom, (top.w, top.h));
        if let Some(t) = trace {
            t(&format!(
                "attempt {}->{} clear={}",
                ctx.from_id,
                ctx.to_id,
                world.clear(&pts)
            ));
        }
        // Sealed-port fallback: no candidate route cleared the world, so
        // retry with the free ports slid along their sides before keeping
        // the defective route.
        if !world.clear(&pts)
            && let Some(alt) = port_fallback_route(ctx, world, &geom, (top.w, top.h), &shared_sides)
        {
            pts = alt;
        }
        pts
    };

    let mut paths: Vec<Option<Vec<(f32, f32)>>> = (0..diagram.edges.len()).map(|_| None).collect();
    for ei in 0..diagram.edges.len() {
        paths[ei] = Some(attempt(&ctxs[ei], &edge_world(&geom, &ctxs[ei])));
    }

    // Phase 2: rip-up-and-reroute until clear, frozen, or bounded. The
    // ripped edges reroute in index order, each against the *live* paths —
    // a later ripped edge sees an earlier one's already-moved segments
    // (two edges that parked on the same slot cannot both keep it; the
    // deterministic order splits them, and the pre-passes make such
    // conflicts rare).
    // Test hook: `phase1_only` skips the repair loop, leaving the
    // simultaneous phase-1 routes in place for permutation-invariance
    // checks. Production layout always runs the full loop.
    let passes = if phase1_only { 0 } else { MAX_RIPUP_PASSES };
    let mut frozen: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for _pass in 0..passes {
        // Each edge's reserved pocket is derived from its *current* polyline
        // (the escape a router would take from each port), so a repair
        // reroute that moved an approach also moves its reservation.
        let pockets: Vec<Vec<(f32, f32, f32, f32)>> = paths
            .iter()
            .map(|p| p.as_ref().map(|pts| pocket_rects(pts)).unwrap_or_default())
            .collect();
        let violators = scan_violations(&paths, &ctxs, &geom, &pockets);
        let rip: std::collections::BTreeSet<usize> =
            violators.difference(&frozen).copied().collect();
        if rip.is_empty() {
            break;
        }
        for &ei in &rip {
            if let Some(t) = trace {
                t(&format!(
                    "pass {}: rip {}->{}",
                    _pass, ctxs[ei].from_id, ctxs[ei].to_id
                ));
            }
            let mut world = edge_world(&geom, &ctxs[ei]);
            for (j, ps) in pockets.iter().enumerate() {
                if j != ei {
                    for &r in ps {
                        world.push_corridor(r);
                    }
                }
            }
            for (j, path) in paths.iter().enumerate() {
                if j == ei {
                    continue;
                }
                if let Some(pts) = path {
                    for w in pts.windows(2) {
                        world.push_soft_segment(w[0], w[1]);
                    }
                }
            }
            // Keep the better of the two routes: the repair pass exists to
            // clear conflicts, not to trade a clean shape for a long detour.
            // A reroute that is not strictly better on the world's own cost
            // model (length + bends + soft occupancy, hard clearance first)
            // leaves the current polyline in place — but only when the old
            // route is itself *clean*: a route riding a peer's band is
            // exactly what this pass was invoked to fix, so it never wins the
            // comparison (otherwise two riders can each "keep" themselves
            // against the other's reroute and the ride survives every pass).
            let candidate = attempt(&ctxs[ei], &world);
            let keep_old = match &paths[ei] {
                Some(old_pts) if world.clear(old_pts) => {
                    let (cand, old) = (
                        route_score(&candidate, &world),
                        route_score(old_pts, &world),
                    );
                    // Keep the current route when the reroute is no shorter
                    // *and* no less conflicting — the pass is here to clear
                    // conflicts, not to shuffle a legal route into a longer
                    // one. A reroute that genuinely reduces soft occupancy
                    // wins even when it costs length (that is the repair).
                    cand >= old - 1e-3
                        && world.soft_cost(&candidate) >= world.soft_cost(old_pts) - 1e-3
                }
                _ => false,
            };
            if !keep_old {
                paths[ei] = Some(candidate);
            }
        }
        // Freeze edges that could not be cleared this pass.
        let still = scan_violations(&paths, &ctxs, &geom, &pockets);
        for &ei in &rip {
            if still.contains(&ei) {
                frozen.insert(ei);
            }
        }
    }
    let mut edges_out: Vec<EdgePath> = paths
        .into_iter()
        .zip(ctxs.iter())
        .map(|(path, ctx)| EdgePath {
            from: ctx.from_id.clone(),
            to: ctx.to_id.clone(),
            points: path.expect("every edge routed"),
            label_at: None,
        })
        .collect();

    // ---- M13: label placement --------------------------------------------
    // Every polyline is routed now, so each label's anchor can be scored
    // against the complete world — other edges' segments, other labels'
    // knockouts, node rects, group titles, and every rendered arrowhead —
    // and stored on its `EdgePath`; the renderer draws the knockout rect +
    // text at the anchor. Group frame borders are deliberately *not*
    // obstacles (M13.1): a label may cross a frame outline.
    let paths: Vec<Vec<(f32, f32)>> = edges_out.iter().map(|e| e.points.clone()).collect();
    let mut label_frames: Vec<(f32, f32, f32, f32)> = Vec::new();
    for s in 0..diagram.groups.len() {
        if let Some(title) = diagram.groups[s].title.as_deref() {
            label_frames.push(title_text_rect(geom.frame(s), title));
        }
    }
    let label_world = LabelWorld {
        nodes: (0..diagram.nodes.len()).map(|gi| geom.node(gi)).collect(),
        frames: label_frames,
        arrows: paths.iter().filter_map(|p| arrowhead_rect(p)).collect(),
    };
    let labels: Vec<Option<&str>> = diagram.edges.iter().map(|e| e.label.as_deref()).collect();
    let anchors = place_labels(&labels, &paths, &label_world);
    for (e, a) in edges_out.iter_mut().zip(anchors) {
        e.label_at = a;
    }
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

    let mut groups_out = Vec::with_capacity(diagram.groups.len());
    for gi in 0..diagram.groups.len() {
        let (x, y, w, h) = geom.frame(gi);
        groups_out.push(GroupRect {
            index: gi,
            x,
            y,
            w,
            h,
        });
    }

    // ---- M11: canvas fit — the page margin clear on every side ------------
    // Edge routing can leave the laid-out content bounds (an excursion
    // around a frame at the canvas edge, an escape past the page margin, a
    // long side run), and a label's knockout can overhang its line. Fit the
    // canvas around *all* drawn geometry with at least [`MARGIN`] of clear
    // space on every side — translating (left/top deficits) and growing
    // (right/bottom) — so nothing ever touches the canvas edge; the
    // renderer's viewBox comes straight from width/height. When routing
    // stays within the content bounds this is a no-op: content sits exactly
    // [`MARGIN`] inside the canvas on every side and the extents match to
    // within float noise (hence the 1e-2 tolerances).
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
    for s in &groups_out {
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
    // M13: a label's knockout can overhang its line (the offset candidates
    // deliberately push it to one side), so the label rects join the extent.
    for (edge, path) in diagram.edges.iter().zip(&edges_out) {
        if let (Some(label), Some((ax, ay))) = (edge.label.as_deref(), path.label_at) {
            let (lw, lh) = label_box(label);
            min_x = min_x.min(ax - lw / 2.0);
            min_y = min_y.min(ay - lh / 2.0);
            max_x = max_x.max(ax + lw / 2.0);
            max_y = max_y.max(ay + lh / 2.0);
        }
    }
    // Left/top deficits: translate everything by the shortfall (uniformly —
    // no relative geometry changes) so those sides clear [`MARGIN`] too.
    // Right/bottom deficits are handled by the canvas growth below, which
    // runs on the shifted extents. Nothing drawn (an empty diagram): the
    // extents are infinite and both shifts stay 0.
    let sx = if min_x < MARGIN - 1e-2 {
        MARGIN - min_x
    } else {
        0.0
    };
    let sy = if min_y < MARGIN - 1e-2 {
        MARGIN - min_y
    } else {
        0.0
    };
    if sx > 0.0 || sy > 0.0 {
        for n in nodes_out.iter_mut() {
            n.x += sx;
            n.y += sy;
        }
        for s in groups_out.iter_mut() {
            s.x += sx;
            s.y += sy;
        }
        for e in edges_out.iter_mut() {
            for p in e.points.iter_mut() {
                p.0 += sx;
                p.1 += sy;
            }
            if let Some(p) = e.label_at.as_mut() {
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
        groups: groups_out,
        width,
        height,
    }
}

//
// ============ M20 — whole-path maze routing ============
//
// The plan (docs/plan-for-whole-path-maze-routing.md): the end state routes
// every edge by one whole-path search from source port to target port over
// a track graph, against one obstacle model, with frame-crossing topology
// as a *search output*. M20.1 lays the foundation without changing any
// routing behavior: the per-edge frame classifier (which frames are solid,
// which are rideable ancestors) and the track-graph machinery needed to
// route against it. The classifier is a pure function of the containment
// tree and the endpoint node indices — precomputed once per edge in
// [`assemble`] and consumed from [`EdgeCtx::frame_world`].

/// The per-edge *base* obstacle world (M20.3): every node interior —
/// including its own endpoints, which a route may touch only at a designated
/// port (so it cannot cut back through either box) — the edge's ancestor
/// frames as thin border bands (crossable only at designated waypoints), its
/// unrelated frames as fully-solid rects, and every group title's text rect.
/// The occupancy lists (hard/soft peer segments, other edges' reserved port
/// corridors) are added by the routing phase in [`assemble`]. The frame
/// world ([`EdgeFrameWorld`]) is the only per-edge input, so every edge kind
/// routes against the same model.
//
// ---- M20.1 constants ----
//
/// How far outside a per-edge frame world the track-graph routing region
/// extends (M20.1): the A* must be able to find *around-the-neighborhood*
/// detours — a route can legitimately leave the ports' bounding span to go
/// around a wall of sibling frames — so the region is the obstacles' own
/// extent pushed out by this margin on every side, clamped to the canvas.
/// This replaces the old ports-and-obstacles bounding box for whole-path
/// searches; [`track_route`]'s own narrow region survives for the ordinary
/// local reroutes.
const AROUND_MARGIN: f32 = 60.0;

/// Routing cost of a perpendicular crossing of an ancestor frame's border
/// band (M20.1, live for every edge since M20.3): the strong preference the
/// cost model needs for crossing each ancestor frame as few times as possible.
/// A few hundred — far above [`BEND_PENALTY`] and a typical detour's length
/// delta, so a whole-path search does not exit a frame through the top and
/// re-enter through the left over a shorter span — and comparable in
/// weight to the soft-occupancy tier ([`SOFT_RIDE_COST`]): one extra
/// crossing of the edge's own ancestor frames is as bad as riding a
/// peer's band.
const FRAME_CROSS_PENALTY: f32 = 300.0;

/// Routing cost of a *designated* crossing of an ancestor frame's border band
/// (M20.4) — a crossing whose segment ends on the band's centerline (a
/// recorded waypoint, which [`seg_clear_border`] exempts).
///
/// The M20.1 model made these free, which was right for *clearance* (a
/// crossing waypoint must be reachable) but wrong for *cost*: an out-and-back
/// detour (exit the frame through its top, walk around, re-enter through a
/// side) then paid exactly as much as one straight descent, so a search that
/// preferred a different column for any reason could leave and re-enter the
/// frame at no penalty — and the frame-crossing fan the pre-pass allocates
/// had no leverage. Charging a tenth of [`FRAME_CROSS_PENALTY`] here keeps the
/// ordering the search needs: a designated crossing is far cheaper than a
/// mid-band transversal (so the search still lands on crossing tracks), but
/// crossing a frame twice now costs visibly more than once (so the search
/// stays on its own assigned column instead of detouring out and back).
const FRAME_RECROSS_COST: f32 = FRAME_CROSS_PENALTY / 10.0;

/// Cost per pixel of a cross-run's offset from the nearest seeded jog row
/// (M20.3). Tiny — a tie-break, not a routing preference: a Z's length is
/// row-invariant, so without it the row is decided by heap order; with it a
/// 50 px drift over a 100 px run costs 1 (a twelfth of a bend), so the row
/// the ladder's Z used wins every tie but a genuinely better shape still
/// wins outright.
const JOG_DRIFT_COST: f32 = 0.02;
/// Cost per pixel of a cross-run's offset from the edge's *assigned* lane
/// (M20.3). Far stronger than [`JOG_DRIFT_COST`] — the lane is the M9 lane
/// pre-pass's separation guarantee, so the search must use it wherever the
/// row is routable — but still a preference: a lane that is blocked (or that
/// would force a long detour) loses to a clean alternative row.
const LANE_DRIFT_COST: f32 = 2.0;
/// How far outside the two ports' flow span a lane row may still be honored
/// (M20.3): enough for a jog just past a port, not enough for a row that
/// belongs to another gap.
const LANE_ROW_SLACK: f32 = 24.0;
/// Cost per pixel of a cross-run's offset from an *assigned frame crossing*
/// (M20.4). The counterpart of [`LANE_DRIFT_COST`] for the ancestor-frame
/// sides the search crosses: the pre-pass ([`separate_frame_crossings`])
/// allocates each shared frame side a set of crossing coordinates, and the
/// search must land on its own one wherever that row is routable — otherwise
/// two edges into one frame side collapse onto one row and the repair loop
/// has to push one of them all the way around the frame. A preference, not a
/// bar: a blocked crossing row loses to a clean alternative.
const CROSSING_DRIFT_COST: f32 = 40.0;

/// The class of one group frame for one edge (M20.1), as
/// [`FrameClass::of_frame`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameClassKind {
    /// On an endpoint's ancestor-or-self chain: perpendicular crossing
    /// legal (a search output, M20.2+), parallel riding barred.
    Ancestor,
    /// Neither endpoint's ancestor: fully solid — interior and border
    /// band; nothing may touch it. (A corridor *between* two solid frames
    /// is open space and legal.)
    Solid,
}

/// The per-edge frame classification (M20.1): a pure function of the
/// containment tree and the two endpoint node indices. The containment
/// tree's only remaining routing role in the whole-path regime (live for
/// every edge since M20.3) is exactly this two-way split:
///
/// - **ancestor frames** — the frames on the ancestor-or-self chains of
///   the two endpoints (the chains [`chain_to_lca`] reads). An edge may
///   cross such a frame's border band perpendicularly at any coordinate
///   of its choosing, and may not ride parallel on the band.
/// - **solid frames** — every group frame that is neither endpoint's
///   ancestor-or-self: sibling groups and unrelated deep subtrees. Fully
///   blocked — interior *and* border band.
///
/// [`FrameClass::of`] is what [`edge_world`] and [`EdgeFrameWorld::build`]
/// consume; the classification tests pin every nesting shape.
#[derive(Debug, Clone, Default)]
struct FrameClass {
    /// Ancestor-or-self frames of either endpoint, sorted by index (the
    /// two chains' union; deterministic, and equal worlds compare equal).
    /// `None` (top level) is not a frame and never appears.
    ancestors: Vec<usize>,
    /// Every other group frame: fully solid, sorted by index.
    solid: Vec<usize>,
    /// True when the two endpoints' chain heads are *disjoint* subtrees
    /// (neither group is an ancestor-or-self of the other) — the
    /// genuinely cross-boundary shape. False when one chain contains the
    /// other (one endpoint nests inside the other's group) or when either
    /// endpoint is top-level (`None` — the top level contains every
    /// group, so no group-vs-group disjointness applies).
    chains_disjoint: bool,
}

/// Does the ancestor-or-self chain of `node_group` (innermost first)
/// contain group frame `g`?
fn chain_contains(groups: &[Group], node_group: Option<usize>, g: usize) -> bool {
    let mut cur = node_group;
    while let Some(idx) = cur {
        if idx == g {
            return true;
        }
        cur = groups[idx].parent;
    }
    false
}

impl FrameClass {
    /// Classify the frames for one edge's endpoints (M20.1). `fg`/`tg` are
    /// the two endpoints' group memberships (`None` = top level).
    fn of(fg: Option<usize>, tg: Option<usize>, groups: &[Group]) -> Self {
        let mut solid: Vec<usize> = (0..groups.len())
            .filter(|&g| !chain_contains(groups, fg, g) && !chain_contains(groups, tg, g))
            .collect();
        let mut ancestors: Vec<usize> = Vec::new();
        let mut cur = fg;
        while let Some(idx) = cur {
            if !ancestors.contains(&idx) {
                ancestors.push(idx);
            }
            cur = groups[idx].parent;
        }
        let mut cur = tg;
        while let Some(idx) = cur {
            if !ancestors.contains(&idx) {
                ancestors.push(idx);
            }
            cur = groups[idx].parent;
        }
        ancestors.sort_unstable();
        solid.sort_unstable();
        Self {
            ancestors,
            solid,
            chains_disjoint: fg.is_some()
                && tg.is_some()
                && fg != tg
                && !chain_contains(groups, fg, tg.unwrap())
                && !chain_contains(groups, tg, fg.unwrap()),
        }
    }

    /// The class of one frame index.
    fn of_frame(&self, g: usize) -> FrameClassKind {
        if self.ancestors.binary_search(&g).is_ok() {
            FrameClassKind::Ancestor
        } else {
            FrameClassKind::Solid
        }
    }
}

/// A per-edge frame world (M20.1): the classification plus the obstacle
/// fields built from it, carried on [`EdgeCtx`] so the routing regime reads
/// one struct instead of re-deriving the per-LCA-context frame list.
#[derive(Debug, Clone, Default)]
struct EdgeFrameWorld {
    /// The classification the fields derive from.
    class: FrameClass,
    /// Thin border bands of every ancestor-or-self frame — the edge's
    /// crossable border field, the LCA-level band field generalized to
    /// the whole ancestor chain.
    ancestor_bands: Vec<(f32, f32, f32, f32)>,
    /// Full rects of the unrelated (solid) frames.
    solid: Vec<(f32, f32, f32, f32)>,
}

impl EdgeFrameWorld {
    /// Build from the laid-out geometry: the classification runs once and
    /// the rect fields resolve here, not per lookup.
    fn build(geom: &Geometry, fg: Option<usize>, tg: Option<usize>) -> Self {
        let class = FrameClass::of(fg, tg, &geom.diagram.groups);
        Self {
            ancestor_bands: class
                .ancestors
                .iter()
                .flat_map(|&g| border_bands(geom.frame(g)))
                .collect(),
            solid: class.solid.iter().map(|&g| geom.frame(g)).collect(),
            class,
        }
    }

    /// Is the polyline clear of the *frame* world alone: every ancestor
    /// band (designated-crossing semantics, [`seg_clear_border`]) and
    /// every solid frame rect (full [`ROUTE_PAD`] clearance,
    /// [`seg_clear_solid`], like a node interior)? The routing phase adds
    /// peer occupancy on top; this predicate is the frame contribution the
    /// whole-path search (M20.2+) scores against.
    fn clear(&self, pts: &[(f32, f32)]) -> bool {
        pts.windows(2).all(|w| self.seg_clear(w[0], w[1]))
    }

    /// One segment against the frame world alone.
    fn seg_clear(&self, p0: (f32, f32), p1: (f32, f32)) -> bool {
        self.ancestor_bands
            .iter()
            .all(|&b| seg_clear_border(p0, p1, b))
            && self.solid.iter().all(|&r| seg_clear_solid(p0, p1, r))
    }

    /// The routing cost of a polyline against the frame world: a
    /// [`FRAME_CROSS_PENALTY`] per mid-segment perpendicular crossing of
    /// an ancestor band. A segment *ending* on a band's centerline is a
    /// designated crossing (a port or a recorded waypoint), not a mid-run
    /// transversal, and is free — matching [`seg_clear_border`]'s
    /// designated-crossing exemption.
    fn cross_cost(&self, pts: &[(f32, f32)]) -> f32 {
        pts.windows(2)
            .map(|w| {
                self.ancestor_bands
                    .iter()
                    .filter(|&&b| band_crossing(w[0], w[1], b).is_some())
                    .count() as f32
                    * FRAME_CROSS_PENALTY
            })
            .sum()
    }

}

/// The widened region a whole-path search routes in (M20.1): the frame
/// world's obstacle extents pushed out by [`AROUND_MARGIN`] on every side
/// (the room an around-the-neighborhood detour needs), clamped to the
/// canvas. Pure and deterministic — a pure function of the frame world,
/// the node rects, and the canvas.
fn around_region(
    fw: &EdgeFrameWorld,
    nodes: &[(f32, f32, f32, f32)],
    canvas: (f32, f32),
) -> (f32, f32, f32, f32) {
    let mut minx = f32::INFINITY;
    let mut miny = f32::INFINITY;
    let mut maxx = f32::NEG_INFINITY;
    let mut maxy = f32::NEG_INFINITY;
    let mut grow = |(x, y, w, h): (f32, f32, f32, f32)| {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x + w);
        maxy = maxy.max(y + h);
    };
    for &r in nodes {
        grow(r);
    }
    for &r in &fw.solid {
        grow(r);
    }
    for &b in &fw.ancestor_bands {
        grow(b);
    }
    if !minx.is_finite() {
        return (0.0, 0.0, canvas.0, canvas.1);
    }
    let x0 = (minx - AROUND_MARGIN).max(0.0);
    let y0 = (miny - AROUND_MARGIN).max(0.0);
    let x1 = (maxx + AROUND_MARGIN).min(canvas.0.max(x0 + 1.0));
    let y1 = (maxy + AROUND_MARGIN).min(canvas.1.max(y0 + 1.0));
    (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
}

/// [`track_route`] with a caller-chosen bounding region (M20.1): identical
/// track seeding (node rects and bands seed their edges ±[`TRACK_OFF`],
/// bands also their centerlines) but the *given* region instead of the
/// derived ports-and-obstacles one, so a whole-path search (M20.2) can look
/// beyond the ports' bounding span to find around-the-neighborhood detours.
/// Returns the polyline, or an empty vec when no route exists in the region
/// (the caller falls back — the bounded convention).
fn track_route_in_region(
    a: (f32, f32),
    b: (f32, f32),
    obs: &Obstacles,
    esc: Escapes,
    region: (f32, f32, f32, f32),
) -> Vec<(f32, f32)> {
    let mut vts = vec![a.0, b.0];
    let mut hts = vec![a.1, b.1];
    for &(x, y, w, h) in obs.nodes.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
    }
    // Title text rects seed tracks just outside themselves, so a descent can
    // pass beside a title and a cross-run can sit just below it.
    for &(x, y, w, h) in obs.titles.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
    }
    for &(x, y, w, h) in obs.borders.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
        if h <= w {
            hts.push(y + h / 2.0);
        } else {
            vts.push(x + w / 2.0);
        }
    }
    // M20.2: the whole-path search's ancestor bands ride the same track
    // seeding as borders (their edges ± TRACK_OFF + their centerlines —
    // the designated-crossing rows); because `push_cross_bands` mirrors
    // them into `borders` they are usually already present, but a
    // hand-built world may seed one list without the other, so both are
    // seeded here (deterministic and order-insensitive — the tracks are
    // sorted and deduped below).
    for &(x, y, w, h) in obs.cross_bands.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
        if h <= w {
            hts.push(y + h / 2.0);
        } else {
            vts.push(x + w / 2.0);
        }
    }
    for &t in obs.jog_rows.iter() {
        vts.push(t);
        hts.push(t);
    }
    // M20.4: the assigned frame crossings seed the track *along* the frame
    // side they cross — a Top/Bottom side's crossing is an x coordinate (a
    // vertical track through the band), a Left/Right side's is a y (a
    // horizontal one). Seeding the wrong axis would drop a track a fraction
    // of a pixel off the band's centerline, splitting a descent into a hop
    // that starts *beside* the centerline and so fails the designated-crossing
    // exemption ([`seg_clear_border`]) — which is exactly the trap that made
    // the search detour around the frame instead of entering at its crossing.
    for &(_, side, c) in obs.crossing_rows.iter() {
        match side {
            Side::Top | Side::Bottom => vts.push(c),
            Side::Left | Side::Right => hts.push(c),
        }
    }
    if let Some(l) = obs.lane_row {
        vts.push(l);
        hts.push(l);
    }
    vts.sort_by(f32::total_cmp);
    hts.sort_by(f32::total_cmp);
    vts.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
    hts.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
    let (rx, ry, rw, rh) = region;
    vts.retain(|&x| x >= rx - 1e-2 && x <= rx + rw + 1e-2);
    hts.retain(|&y| y >= ry - 1e-2 && y <= ry + rh + 1e-2);
    let nv = vts.len();
    let nh = hts.len();
    if nv == 0 || nh == 0 {
        return Vec::new();
    }
    // The ports must survive the region clip (the caller's region is built
    // to contain them; a degenerate region that lost a port routes nothing
    // and the caller falls back, the bounded convention).
    let (Some(ai), Some(aj), Some(bi), Some(bj)) = (
        vts.iter().position(|&x| (x - a.0).abs() < 1e-2),
        hts.iter().position(|&y| (y - a.1).abs() < 1e-2),
        vts.iter().position(|&x| (x - b.0).abs() < 1e-2),
        hts.iter().position(|&y| (y - b.1).abs() < 1e-2),
    ) else {
        return Vec::new();
    };
    track_search(
        (ai, aj),
        (bi, bj),
        &vts,
        &hts,
        obs,
        esc,
        (rx, ry, rx + rw, ry + rh),
    )
}

/// The whole-path search (M20.2, live for every edge from M20.3): one A* from
/// source port to target port
/// over a track graph against the full obstacle model — the widened
/// [`around_region`] box, the per-edge frame world's ancestor bands (as
/// `borders`: parallel riding barred, perpendicular crossings at recorded
/// waypoints) and its solid frames (as node rects: fully blocked), with
/// the [`FRAME_CROSS_PENALTY`] charged per mid-run ancestor transversal so
/// the search crosses each ancestor frame as few times as possible. The
/// graph's cost model is segment length + bends + soft peer occupancy +
/// frame crossings.
///
/// `esc` carries the endpoint escape stubs (the outward port escapes) so
/// the search never draws a spike back over them; `nodes` must hold every
/// node rect (the region's other extent input) and `canvas` the page
/// size. Returns the polyline, or an empty vec when no route exists in
/// the region (the caller falls back — the bounded convention).
///
/// This is the M20 router: [`route_edge`] pushes its result *first* for every
/// unforced edge — direct (M20.2) and cross-boundary (M20.3) alike — so the
/// search, not a piecewise ladder, is what those edges are drawn from. It
/// re-derives the flat engine's shapes because the caller seeds the engine's
/// own jog rows ([`Obstacles::jog_rows`]); the ladder's candidates remain
/// behind it as the bounded fallback for a region where no route exists.
fn whole_path_route(
    a: (f32, f32),
    b: (f32, f32),
    fw: &EdgeFrameWorld,
    obs: &Obstacles,
    esc: Escapes,
    nodes: &[(f32, f32, f32, f32)],
    canvas: (f32, f32),
) -> Vec<(f32, f32)> {
    // The world: what the caller's obstacle model knows plus the frame
    // world's ancestor bands (as borders + the crossing cost) and its
    // solid frames (as node rects). The caller usually already built both
    // ([`edge_world`]); pushing them again is idempotent for clearance and
    // keeps a hand-built world (unit fixtures) correct.
    let mut world = obs.clone();
    world.port_graph = true;
    world.push_cross_bands(&fw.ancestor_bands);
    for &r in &fw.solid {
        world.push_node(r);
    }
    let region = around_region(fw, nodes, canvas);
    let pts = track_route_in_region(a, b, &world, esc, region);
    if pts.is_empty() {
        return pts;
    }
    // Re-record the ancestor crossings the polyline collapse may have
    // folded away: a straight run hopping across a band's centerline
    // collapses to one segment through the band, which the clearance
    // check would bar — the waypoint keeps it a designated crossing
    // (the same convention [`track_route`] applies to its own bands).
    record_border_crossings(&pts, &fw.ancestor_bands)
}

/// The per-edge base obstacle world (M20): every node interior, the edge's
/// ancestor-frame border bands (crossable at recorded waypoints, never
/// ridden parallel), its unrelated frames as fully-solid rects, and every
/// group title's text rect. The M20 model is the same for every edge kind —
/// the frame world ([`EdgeFrameWorld`]) is the only per-edge input, so the
/// dispatch no longer varies the world by edge kind (M20.3 retired the
/// cross-boundary regime's LCA-level band field).
/// One edge's whole-path search candidate (M20.2/M20.3), or `None` when the
/// search found nothing that clears the hard world (the caller's ladder then
/// wins — the bounded convention).
///
/// The search runs node-port to node-port for every edge — the plan's "one
/// polyline from port to port". For a cross-boundary edge that means the
/// search owns the whole span: the within-frame stubs, the chain frames'
/// crossings, and the LCA-level run are all search outputs, and the
/// containment tree's only remaining routing role is the ancestor/solid
/// classification.
fn whole_path_candidate(
    ctx: &EdgeCtx,
    geom: &Geometry,
    world: &Obstacles,
    canvas: (f32, f32),
) -> Option<Vec<(f32, f32)>> {
    let fw = ctx.frame_world(geom);
    let nodes: Vec<(f32, f32, f32, f32)> = (0..geom.diagram.nodes.len())
        .map(|gi| geom.node(gi))
        .collect();
    let a = ctx.from_port;
    let b = ctx.to_port;
    let esc = Escapes {
        start: Some(Dir::of_side(ctx.from_side)),
        end: Some(Dir::of_side(ctx.to_side)),
    };
    // The flat engine's jog rows (direct edges) and the assigned LCA lane
    // (cross-boundary edges) seed the search's tracks, so it re-derives the
    // shape the ladder drew instead of picking a track arbitrarily.
    let mut wj = world.clone();
    wj.jog_axis = ctx.flow;
    // M20.4: the frame-side crossing fan the pre-pass allocated for this edge
    // ([`separate_frame_crossings`]). Seeded as tracks and preferred by
    // [`CROSSING_DRIFT_COST`] so the search lands on its own crossing row.
    wj.crossing_rows = ctx.crossing_rows.clone();
    // The M9 lane pre-pass's row is a *preference*, and only for a cross-run
    // that actually lies between the edge's own ports: the lane is computed
    // from the two *node* ports (M20.3), but a row can still sit past an
    // endpoint — a stale row would pull the route into an out-and-back
    // spike — so the search honors it only where it is geometrically
    // meaningful. (M20.4 re-grounded the lane's *inputs* on the node ports
    // and added the frame-crossing fan alongside it; see
    // [`separate_frame_crossings`].)
    let (fa, fb) = (cross_flow(a, ctx.flow).1, cross_flow(b, ctx.flow).1);
    let (lo, hi) = (fa.min(fb) - LANE_ROW_SLACK, fa.max(fb) + LANE_ROW_SLACK);
    wj.lane_row = (!ctx.is_direct && (lo..=hi).contains(&ctx.lane)).then_some(ctx.lane);
    wj.jog_rows = if ctx.is_direct {
        ctx.raw
            .windows(2)
            .map(|w| jog_flow(cross_flow(w[0], ctx.flow).1, cross_flow(w[1], ctx.flow).1))
            .collect()
    } else {
        // A cross-boundary edge's natural rows are the ones the deleted
        // ladder's Z's used: the flow midpoints between consecutive *gates*
        // along the run — the node port, each chain frame's facing border,
        // the rep ports, the other node port. Seeding them lets the search
        // reproduce that shape where it is legal, while a blocked row still
        // lets it choose another. The lane row carries its own much stronger
        // weight ([`Obstacles::lane_row`]).
        let flow = |p: (f32, f32)| cross_flow(p, ctx.flow).1;
        let mut gates = vec![flow(a)];
        for &g in &ctx.from_chain {
            let f = geom.frame(g);
            gates.push(match ctx.from_side {
                Side::Top => f.1,
                Side::Bottom => f.1 + f.3,
                Side::Left => f.0,
                Side::Right => f.0 + f.2,
            });
        }
        for &g in &ctx.to_chain {
            let f = geom.frame(g);
            gates.push(match ctx.to_side {
                Side::Top => f.1,
                Side::Bottom => f.1 + f.3,
                Side::Left => f.0,
                Side::Right => f.0 + f.2,
            });
        }
        gates.push(flow(b));
        let mut rows: Vec<f32> = gates
            .windows(2)
            .map(|w| jog_flow(w[0].min(w[1]), w[0].max(w[1])))
            .collect();
        rows.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
        rows
    };
    let search = whole_path_route(a, b, &fw, &wj, esc, &nodes, canvas);
    if std::env::var("DIAG_Q").is_ok() && ctx.from_id == "s2" {
        let ideal = [(a.0, a.1), (a.0, 95.75), (160.92911, 95.75), (160.92911, 129.5), (160.92911, 155.5), (b.0, 155.5), (b.0, b.1)];
        eprintln!("[q] ideal clear={} score={}", wj.clear(&ideal), route_score(&ideal, &wj));
        eprintln!("[q] found clear={} score={} pts={:?}", wj.clear(&search), route_score(&search, &wj), search);
        eprintln!("[q] jog={:?} lane={:?} cross={:?}", wj.jog_rows, wj.lane_row, wj.crossing_rows);
        let region = around_region(&fw, &nodes, canvas);
        eprintln!("[q] region={region:?}");
        // reachability of the crossing corner
        for &(x, y) in &[(160.92911f32, 95.75f32), (160.92911, 129.5f32), (160.92911, 155.5f32)] {
            eprintln!("[q] ({x:.2},{y:.2}) clear-from-source? seg(150.93,95.75)->({x:.2},{y:.2})={}", wj.seg_clear((150.92911,95.75),(x,y)));
        }
    }
    if search.is_empty() || !world.clear(&search) {
        return None;
    }
    Some(search)
}

fn edge_world(geom: &Geometry, ctx: &EdgeCtx) -> Obstacles {
    let mut obs = Obstacles::empty();
    for (gi, _) in geom.diagram.nodes.iter().enumerate() {
        obs.push_node(geom.node(gi));
    }
    let fw = ctx.frame_world(geom);
    for &r in &fw.solid {
        obs.nodes.push(r);
    }
    for s in 0..geom.diagram.groups.len() {
        if let Some(t) = geom.title_obstacle_rect(s) {
            obs.push_title(t);
        }
    }
    obs.push_cross_bands(&fw.ancestor_bands);
    obs
}

/// Route one edge: the single dispatch point (M11.5). Candidate generators
/// run in priority order — each decides for itself whether it applies — and
/// the least-soft-conflicting candidate that clears the hard obstacle world
/// wins (first-clean wins outright; earliest wins ties); the last candidate
/// is terminal (kept unconditionally: its internal ladder already ends in a
/// route that keeps the ports — the bounded-ladder convention). The special
/// cases keep their fallbacks; the point of the hoist is one dispatch, not
/// fewer cases.
fn route_edge(
    ctx: &EdgeCtx,
    world: &Obstacles,
    geom: &Geometry,
    canvas: (f32, f32),
) -> Vec<(f32, f32)> {
    let mut candidates: Vec<Vec<(f32, f32)>> = Vec::new();
    // M20.3: the whole-path search is the router for every non-loop edge —
    // direct (M20.2) and cross-boundary alike. It is pushed *first* (below,
    // once the ladder's fallbacks are known), so its route wins whenever it
    // clears the hard world; the ladder's candidates stay behind it as the
    // bounded fallback for a region where no route exists.
    //
    // Scope (settled at M20.4): a forced *direct* edge keeps the M11 ladder as
    // its router. The plan's sketch ("the forced port is simply where the
    // search begins") was tried and rejected — the search's cost model has no
    // term for "leave the box along the requested side by [`SIDE_ESCAPE`]
    // before anything else", so grounding a forced direct edge on it drew
    // shorter but *wrong* shapes (a forced `from="top"` escape became a stub
    // barely longer than the arrowhead, and `examples/sides.dgmr`'s forced
    // excursions lost their deliberate clearance). M11's escape-and-ladder
    // already produces exactly the shape the attribute asks for, and it is
    // the only router that encodes the request; a forced *cross-boundary*
    // edge does search (the search simply starts/ends at the forced ports and
    // the escape bars keep it from cutting back through either box).
    let search_first = !ctx.is_self_loop;
    let search_applies = !ctx.is_direct || (ctx.forced_from.is_none() && ctx.forced_to.is_none());
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
            // had. The M11 escape-based whole-edge route (ports escape
            // outward along their sides, then the ladder) is the alternate:
            // when the flat trunk is clear but *conflicts* (rides a peer's
            // soft band — two aligned chains converge on one line), the
            // separating jog must exist as a candidate to win the cost
            // comparison; the flat waypoints alone would freeze the ride.
            candidates.push(direct_chain_route(&ctx.raw, ctx.flow, world));
            candidates.push(forced_direct_path(
                ctx.from_port,
                ctx.from_side,
                ctx.to_port,
                ctx.to_side,
                ctx.flow,
                world,
            ));
        }
    } else {
        // Cross-boundary (M20.3): one whole-path search from node port to
        // node port (pushed below), with the frame-crossing topology as a
        // search output. The rep model — rep ports, frame-crossing fans,
        // within-frame stubs, title detours, the around-target wrapper, and
        // the M14 coupled/decoupled duality — is deleted: the containment
        // tree's only routing role is the ancestor/solid classification.
        //
        // The escape-and-ladder route is the terminal fallback: it always
        // returns something that keeps the ports, so routing never fails even
        // when no whole-path route exists in the region (the bounded-ladder
        // convention). For a forced edge it is also the *only* router — a
        // forced side binds the node port with M11 semantics (port on the
        // requested side, then an outward [`SIDE_ESCAPE`]), which the
        // search's free-choice port model cannot honor (see [`route_edge`]'s
        // scope note: grounding forced sides on the search was tried at M20.4
        // and rejected).
        candidates.push(forced_cross_boundary_path(ctx, world));
    }
    // The whole-path search (M20.2 direct, M20.3 cross-boundary): pushed
    // *first*, so its route wins whenever it clears the hard world. It
    // re-derives the shapes the ladder drew (the flat engine's jog rows are
    // seeded, see [`Obstacles::jog_rows`]) and is the *only* router with a
    // whole-path view of the M20 obstacle model: solid unrelated frames are
    // fully blocked, ancestor bands are crossable at a per-crossing penalty,
    // titles are solid, and soft peers are negotiable.
    //
    // Scope: `search_applies` above — every unforced edge, plus a forced
    // cross-boundary edge (whose forced ports are where the search starts and
    // ends). A forced *direct* edge keeps the M11 ladder (see the scope note
    // at the top of this function).
    if search_first
        && search_applies
        && let Some(search) = whole_path_candidate(ctx, geom, world, canvas)
    {
        candidates.insert(0, search);
    }
    // Accept (M16): the first candidate that clears the hard world *and*
    // carries no soft conflict; otherwise the least-conflicting hard-clear
    // candidate (earliest on ties — the generator priority breaks them);
    // otherwise the terminal candidate, kept unconditionally (the
    // bounded-ladder convention).
    let mut best: Option<(usize, f32)> = None;
    for (idx, p) in candidates.iter().enumerate() {
        if !world.clear(p) {
            continue;
        }
        let c = world.soft_cost(p);
        if c < 1e-6 {
            return p.clone();
        }
        if best.as_ref().is_none_or(|(_, bc)| c < bc - 1e-6) {
            best = Some((idx, c));
        }
    }
    match best {
        Some((idx, _)) => candidates[idx].clone(),
        None => candidates.last().expect("at least one candidate").clone(),
    }
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
            // straight (waypoints preserved) — unless the leg is blocked (a
            // peer edge's parked trunk sharing the channel, a node the flat
            // engine's aligned placement ran into), in which case the track
            // graph finds the jog around it. Without this check, two aligned
            // chains converging on one vertical line would draw on top of
            // each other forever: no Z is emitted for an aligned pair, so
            // the ladder never runs.
            let seg = [p0, p1];
            if obs.clear(&seg) {
                out.push(p1);
            } else {
                let tr = track_route(p0, p1, obs, Escapes::default());
                if !tr.is_empty() && obs.clear(&tr) {
                    out.extend(tr.into_iter().skip(1));
                } else {
                    // Last resort: keep the straight leg (the bounded
                    // convention — the scan's freeze accepts the residue).
                    out.push(p1);
                }
            }
        } else {
            let jf = jog_flow(f0, f1);
            out.extend(route_lca(p0, p1, axis, jf, obs, Escapes::default()));
        }
    }
    dedup_consecutive(&mut out);
    out
}

// ---- Sealed-port fallback: sliding a lone side's port (post-M13) ------
//
// The M11.5 ladder's known failure (recorded in `docs/milestones.md`): a
// node port's escape pocket can be sealed inside the padded bands of
// already-routed peer segments — any routed L-corner within
// [`SEGMENT_OBSTACLE_PAD`] of a port escape seals it — so [`track_route`]
// finds nothing and the terminal Z is kept even though it cuts through a
// node (the `api -- from="top" --> db` and `bus -- from="right" to="left"
// --> queue` shapes of `examples/sides.dgmr`). When that happens (no
// candidate route clears the obstacle world), the edge retries with its
// free ports slid along their sides: a port's escape pocket moves with it,
// so a side whose middle is sealed often still routes from near a corner.
// The M11.5 verdict calls for "a reserved approach right-of-way for every
// node port" in the eventual global routing pass; this is the cheap local
// version of that guarantee. Like the fan ([`separate_ports`]), it only
// moves a lone side's port — a shared side's ports are the fan's to place —
// and it never moves a self-loop's port.

/// Candidate cross-coordinates for one edge end that may slide along its
/// side: the current one first (a lone side's centre), then the two
/// [`PORT_INSET`] corner ports — the same usable-span ends a fanned side
/// draws between (the "7 px from the corner" positions) — the one nearer
/// the far endpoint's cross first (the edge leans toward its partner). A
/// side with no room to move yields just the current coordinate. The order
/// is the try order: alternatives are attempted in it, so the fallback
/// keeps the smallest deviation that routes.
fn port_alternates(
    rect: (f32, f32, f32, f32),
    side: Side,
    current: f32,
    other_cross: f32,
) -> Vec<f32> {
    let (lo, hi) = side_cross_span(rect, side);
    let (a, b) = (lo + PORT_INSET, hi - PORT_INSET);
    if b <= a + 1e-3 {
        return vec![current];
    }
    let (near, far) = if (a - other_cross).abs() <= (b - other_cross).abs() {
        (a, b)
    } else {
        (b, a)
    };
    let mut out = vec![current];
    for c in [near, far] {
        if out.iter().all(|&o| (o - c).abs() > 1e-3) {
            out.push(c);
        }
    }
    out
}

/// A copy of `ctx` with the given end(s) moved to new cross-coordinates
/// along their (unchanged) sides — the same rewrite the port-separation
/// fill in [`assemble`] applies ([`port_at_cross`] for the port points,
/// [`set_cross`] on a direct edge's flat-waypoint ends), so every candidate
/// generator sees a consistent context. A direct edge's flat waypoint is
/// rewritten only when it lies on the effective side itself: under a
/// contradictory forced side (M11) the router reads the port points, not
/// the raw waypoints, and the raw port's side may even carry the other
/// cross axis.
fn ctx_with_crosses(
    ctx: &EdgeCtx,
    _geom: &Geometry,
    from: Option<f32>,
    to: Option<f32>,
) -> EdgeCtx {
    let mut v = ctx.clone();
    if let Some(c) = from {
        v.from_port = port_at_cross(v.from_rect, v.from_side, c);
        v.from_cross = cross_of(v.from_port, v.from_side);
        if v.is_direct
            && let Some(&p) = v.raw.first()
            && side_of_port(v.from_rect, p) == v.from_side
        {
            v.raw[0] = set_cross(p, v.from_side, v.from_cross);
        }
    }
    if let Some(c) = to {
        v.to_port = port_at_cross(v.to_rect, v.to_side, c);
        v.to_cross = cross_of(v.to_port, v.to_side);
        if v.is_direct
            && let Some(&p) = v.raw.last()
            && side_of_port(v.to_rect, p) == v.to_side
        {
            let n = v.raw.len();
            v.raw[n - 1] = set_cross(p, v.to_side, v.to_cross);
        }
    }
    v
}

/// The sealed-port fallback route: when [`route_edge`] leaves `ctx`'s edge
/// with a route that does not clear the obstacle world (no candidate route
/// found — see the section note), retry the edge with its free ends slid
/// along their sides ([`port_alternates`]) and return the first alternative
/// whose route clears. A free end is one this fallback may move: not a
/// self-loop's, and not a side shared with other edges ([`separate_ports`]
/// already placed those). Alternatives are tried one
/// end at a time before both together (minimal deviation first, target end
/// before source within a count — a fixed order, so the fallback stays
/// deterministic). `None` — no end may move, or no alternative routes
/// clear — leaves the caller's route untouched: the fallback only ever
/// replaces a defective route with a clear one, never trades one defect
/// for another.
fn port_fallback_route(
    ctx: &EdgeCtx,
    world: &Obstacles,
    geom: &Geometry,
    canvas: (f32, f32),
    shared_sides: &std::collections::HashSet<(usize, Side)>,
) -> Option<Vec<(f32, f32)>> {
    if ctx.is_self_loop {
        return None;
    }
    let from_free = !shared_sides.contains(&(ctx.fi, ctx.from_side));
    let to_free = !shared_sides.contains(&(ctx.ti, ctx.to_side));
    if !from_free && !to_free {
        return None;
    }
    let from_alts = if from_free {
        port_alternates(
            ctx.from_rect,
            ctx.from_side,
            ctx.from_cross,
            cross_of(center(ctx.to_rect), ctx.from_side),
        )
    } else {
        vec![ctx.from_cross]
    };
    let to_alts = if to_free {
        port_alternates(
            ctx.to_rect,
            ctx.to_side,
            ctx.to_cross,
            cross_of(center(ctx.from_rect), ctx.to_side),
        )
    } else {
        vec![ctx.to_cross]
    };
    let mut combos: Vec<(usize, usize)> = Vec::new();
    for i in 0..from_alts.len() {
        for j in 0..to_alts.len() {
            if i + j > 0 {
                combos.push((i, j));
            }
        }
    }
    combos.sort_by_key(|&(i, j)| (i + j, i, j));
    for (i, j) in combos {
        let v = ctx_with_crosses(
            ctx,
            geom,
            (i > 0).then_some(from_alts[i]),
            (j > 0).then_some(to_alts[j]),
        );
        let pts = route_edge(&v, world, geom, canvas);
        if world.clear(&pts) {
            return Some(pts);
        }
    }
    None
}

/// Lay out one level (the top level `None`, or a group index) and all of
/// its descendants. Returns the level's contents in its local coordinate
/// system.
fn layout_level(
    diagram: &Diagram,
    level: Option<usize>,
    dir: Direction,
    edge_infos: &[EdgeInfo],
) -> LevelOut {
    // Direct-child groups of this level.
    let child_subs: Vec<usize> = diagram
        .groups
        .iter()
        .enumerate()
        .filter(|(_, sg)| sg.parent == level)
        .map(|(i, _)| i)
        .collect();

    // Recurse into each child group first (bottom-up sizing).
    let mut child_outs: std::collections::HashMap<usize, ChildOut> =
        std::collections::HashMap::new();
    for &cs in &child_subs {
        let child_dir = diagram.groups[cs].direction.unwrap_or(dir);
        let out = layout_level(diagram, Some(cs), child_dir, edge_infos);
        let (top_inset, _bot_pad) = frame_insets(&diagram.groups[cs]);
        let inner_ox = FRAME_PAD_X;
        let inner_oy = top_inset;
        child_outs.insert(
            cs,
            ChildOut {
                out,
                inner_ox,
                inner_oy,
            },
        );
    }

    // Build this level's flat items: direct-child real nodes, then child
    // groups (as opaque compound boxes).
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
        let (top_inset, bot_pad) = frame_insets(&diagram.groups[cs]);
        // The frame is at least wide enough for its contents plus side
        // padding, but no narrower than its title: the title starts at
        // [`FRAME_TITLE_X`] (the renderer anchors it there, so its left
        // edge cannot move) and gets [`title_clear_gap`] past its measured
        // right edge — the same fallback-safe margin the detour entries
        // use, so a viewer's wider fallback face cannot spill the title out
        // of the frame (a measured title can render ~16% wider when the
        // font stack resolves to a face other than the measured one).
        let title_w = diagram.groups[cs]
            .title
            .as_ref()
            .map(|t| text::measure(t, FRAME_TITLE_FONT_SIZE).width)
            .unwrap_or(0.0);
        // A titled frame is also wide enough to pass a route beside its
        // title: the title's routing obstacle (M20.3) extends
        // [`title_clear_gap`] past the measured text, so without the extra
        // [`FRAME_PAD_X`] the obstacle would reach the frame's right border
        // and no within-frame corridor would exist beside the title — every
        // cross-boundary descent would be forced around the outside of the
        // frame. Growing the width keeps the children anchored at the left
        // inset, so the group's internal arrangement is untouched.
        let frame_w = (child.out.w + 2.0 * FRAME_PAD_X)
            .max(FRAME_TITLE_X + title_w + title_clear_gap(title_w) + FRAME_PAD_X);
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

    // Place items. Real nodes record their rect; group boxes record the
    // frame rect and then translate their already-computed contents into
    // this level's local space.
    for (item_idx, &(rx, ry, rw, rh)) in flat.rects.iter().enumerate() {
        match item_refs[item_idx] {
            ItemRef::Node(gi) => out.nodes.push((gi, rx, ry, rw, rh)),
            ItemRef::Sub(cs) => {
                out.frames.push((cs, rx, ry, rw, rh));
                let child = child_outs
                    .remove(&cs)
                    .expect("child out present for its item");
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
fn ancestors(group: Option<usize>, groups: &[crate::ast::Group]) -> Vec<Option<usize>> {
    let mut out = Vec::new();
    let mut cur = group;
    while let Some(idx) = cur {
        out.push(cur);
        cur = groups[idx].parent;
    }
    out.push(None);
    out
}

/// Lowest common ancestor level of two nodes. `None` means the top level.
fn lca_level(a: Option<usize>, b: Option<usize>, groups: &[crate::ast::Group]) -> Option<usize> {
    let ca = ancestors(a, groups);
    let cb = ancestors(b, groups);
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
        if diagram.groups[idx].parent == level {
            return ItemRef::Sub(idx);
        }
        cur = diagram.groups[idx].parent;
    }
    // `level` was not an ancestor of the node — should be unreachable.
    ItemRef::Node(node_idx)
}

/// The group frames containing `node_group`, innermost first, up to (but
/// excluding) `lca` — i.e. the frames a cross-boundary edge crosses on this
/// side. An empty result means `node_group == lca`, so the node is a direct
/// child of the LCA and is its own representative.
fn chain_to_lca(node_group: Option<usize>, lca: Option<usize>, groups: &[Group]) -> Vec<usize> {
    let mut chain = Vec::new();
    let mut cur = node_group;
    while cur != lca {
        match cur {
            Some(idx) => {
                chain.push(idx);
                cur = groups[idx].parent;
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
        Some(idx) => diagram.groups[idx]
            .direction
            .unwrap_or_else(|| effective_direction(diagram.groups[idx].parent, diagram)),
    }
}

/// `(top_inset, bottom_padding)` for a child group's frame.
///
/// The top inset is a one-line title band ([`FRAME_TITLE_H`]) plus one
/// [`text::line_height`] per *additional* title line, plus
/// [`TITLE_BAND_PAD`] — the strip below the title a whole-path route can pass
/// through (M20.3), so the search never has to cross a title. The bottom
/// inset is the plain frame padding.
/// title's stacked lines stay inside the band.
fn frame_insets(sg: &Group) -> (f32, f32) {
    let top = match &sg.title {
        Some(t) => {
            FRAME_TITLE_H
                + (text::line_count(t) - 1) as f32 * text::line_height(FRAME_TITLE_FONT_SIZE)
                // M20.3: the title band keeps a route-width strip below the
                // text, so a within-frame run can pass *under* the title
                // instead of being forced through it. The deleted title
                // detour hand-built a jog in exactly this strip; the search
                // needs the room to exist, then finds it on its own.
                + TITLE_BAND_PAD
        }
        None => FRAME_PAD_Y,
    };
    (top, FRAME_PAD_Y)
}

// ============ Cross-boundary edge routing (M5) ============

/// One of the four sides of an axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// The point on `rect`'s `side` at the given cross-axis coordinate
/// (clamped to that side's span), rather than at the rect's center. For a
/// vertical flow (top-down / bottom-up) the cross axis is `x` and the sides
/// are [`Side::Top`] / [`Side::Bottom`]; for a horizontal flow it is `y` and
/// the sides are [`Side::Left`] / [`Side::Right`]. The M9 port fan and the
/// sealed-port fallback place ports with it.
fn port_at_cross(rect: (f32, f32, f32, f32), side: Side, cross: f32) -> (f32, f32) {
    let (x, y, w, h) = rect;
    match side {
        Side::Top => (cross.clamp(x, x + w), y),
        Side::Bottom => (cross.clamp(x, x + w), y + h),
        Side::Left => (x, cross.clamp(y, y + h)),
        Side::Right => (x + w, cross.clamp(y, y + h)),
    }
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
fn segment_intersects_rect(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
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
/// of node interiors and titles). The whole-path search seeds this row as a
/// track preference (M20.3), so the shapes the flat engine's Z's drew
/// survive wherever they are legal.
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

// ============ M9 — edge separation & obstacle-aware LCA routing ============
//
// Two failure modes the M7 midpoint-jog orthogonalizer could not avoid:
//
// 1. An edge passing *through* a node. The LCA-level segment of a cross-
//    boundary edge runs between two representatives' ports; its midpoint jog
//    can land inside a third node that sits between them (e.g. `User --> VPN`
//    whose jog crosses `prd`, making it look as if `prd` connects to `VPN`).
// 2. Parallel edges laying *on top of* each other. Several edges from one
//    source to one target group share the source's single port and the
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
// segment is rerouted. The M5 headline guarantee (a group's internal
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
/// Shortest terminal leg (port stub) the whole-path search prefers (M20.2):
/// it must exceed the arrowhead's back-extent (`render`'s `markerHeight`, 10)
/// or the final approach line cuts into the arrowhead's side.
const MIN_TERMINAL_STUB: f32 = 12.0;
/// Cost per pixel a terminal leg falls short of [`MIN_TERMINAL_STUB`]
/// (M20.2) — a *preference*, never a bar. A Z's length is jog-row-invariant,
/// so every row ties on length and bends; without this the search jogs one
/// track gap off the target (an 8 px stub, under the arrowhead) instead of a
/// legible mid-gap jog.
const STUB_SHORTFALL_COST: f32 = 25.0;
/// How far a forced-side route escapes outward from a port (along the
/// side's outward normal) before orthogonal routing begins (M11). Sized to
/// clear the arrowhead's back-extent (`markerHeight`, 10) comfortably, and
/// — being just past the top-level page margin ([`MARGIN`], 20) — to push a
/// forced excursion at the canvas edge outside it, where the canvas fit in
/// [`assemble`] grows the canvas around it (keeping [`MARGIN`] clear) instead
/// of silently clipping.
const SIDE_ESCAPE: f32 = 24.0;
/// How far a forced self-loop's excursion runs beyond the node's sides
/// (M11), before wrapping around to the entry side.
const LOOP_OUT: f32 = 18.0;
/// How much clearance an edge's segment keeps around itself when edges
/// negotiate (M9/M16): a segment's padded rect overlaps a *parallel* peer
/// segment (within this much) is a conflict — the two negotiate (M16) rather
/// than one riding on the other. Wide enough that two routes never read as
/// touching (a 1.5 px stroke plus breathing room), but below [`FAN_SEP`]
/// (10 px) so legitimate parallel fan lanes still clear.
const SEGMENT_OBSTACLE_PAD: f32 = 8.0;

// ============ M16 — global routing pass ============
//
// The M11.5 verdict (recorded in `docs/milestones.md`): the one-pass regime
// satisfies the invariants only through per-case fallbacks, because it
// cannot arbitrate a resource conflict — an already-routed peer segment's
// hard band can seal a later edge's port escape pocket, and whose route
// yields then depends on declaration order. The cure it names is a global
// routing pass with three properties, all implemented here:
//
// 1. **Soft peer occupancy.** Peer edge segments are *negotiable*, not hard
//    bands: an edge may route over one (a conflict), and the conflict is
//    resolved by rip-up-and-reroute — never by forcing the current edge into
//    a worse shape to dodge an earlier edge's parked band.
// 2. **Reserved port right-of-way.** Every endpoint node port reserves its
//    approach corridor — the swept rect of the port's escape path
//    ([`escape_point`] outward by [`PORT_RIGHT_OF_WAY`], grown by
//    [`PORT_CORRIDOR_PAD`]) — as a *hard* obstacle for peers: a port must
//    stay reachable. Peers may cross a corridor perpendicularly (crossing
//    edges is legal); running parallel inside one is a violation that rips
//    the invader up and sends it around.
// 3. **Simultaneous assignment + rip-up.** Routing is two phases: a
//    simultaneous pass, where every edge routes against the same base world
//    of nodes and frame borders — no peer segments and no corridors (port
//    pockets don't exist until routes do), so each edge's route is its
//    unconstrained best and independent of declaration order — followed by a
//    bounded, deterministic repair loop
//    that scans the routed set for hard violations (a segment through a
//    node, riding a frame outline, riding a peer's segment, or riding
//    inside a peer's reserved corridor), rips up the involved edges, and
//    reroutes them against a world where the surviving peers' segments are
//    soft (a routing cost, not a barrier). Edges that cannot be fixed get
//    frozen (their violation is the bounded best effort, like the ladder's
//    kept-unconditional terminal).

// ============ M11.5 — one obstacle world, one routing regime ============
//
// Before this milestone the routing regime differed by edge kind, with
// different guarantees each: direct edges orthogonalized blind (no obstacle
// check at all — only forced sides got the ladder), cross-boundary edges
// routed obstacle-aware against node rects only, and forced edges alone
// saw already-routed peer edges as obstacles. The special-case gates
// (around-target, title-detour, forced stubs) were scattered as booleans
// across `assemble` and the stub builder, and the per-edge context was
// derived twice. M11.5 restructures this into ONE regime: the same
// [`Obstacles`] world and the same [`route_lca`] ladder for every edge,
// with edge-type knowledge confined to which obstacles and candidate
// routes each edge feeds it (the candidate generators + one dispatch
// point, [`route_edge`]). M20.3 later replaced the candidate generators with
// the one whole-path search (see the module docs); the world, the
// `(cross, flow)` vocabulary, and the bounded-fallback convention survive.

/// Half-thickness of the thin band that stands for a group frame's drawn
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
/// Clearance a route keeps around a group title's text rect (M20.3): titles
/// are solid obstacles for the whole-path search, so the search routes around
/// them where the deleted title-detour geometry used to jog. Small — a couple
/// of pixels of stroke plus breathing room — so a descent beside the title
/// stays tight to it.
const TITLE_OBSTACLE_PAD: f32 = 1.0;

/// How far outward a node port's reserved approach corridor runs (M16): the
/// swept rect of `port → escape_point(port, side, PORT_RIGHT_OF_WAY)` — the
/// same escape the M11 forced-side routers take — so the corridor is exactly
/// the approach any router would use first.
const PORT_RIGHT_OF_WAY: f32 = SIDE_ESCAPE;
/// How much slack a reserved port corridor keeps around its escape path
/// (M16). Small: a corridor bars only parallel riding inside it, and fan
/// lanes/port fans legitimately run [`FAN_SEP`] (10 px) from each other, so
/// a ±3 px band around one edge's escape never touches a sibling's.
const PORT_CORRIDOR_PAD: f32 = 3.0;
/// Routing cost of a segment riding inside one soft peer-segment band
/// (M16): a conflict, not a barrier — the repair loop negotiates it away by
/// ripping up one of the two edges. Sized far above [`BEND_PENALTY`] (12)
/// so a shortest conflict-free detour always beats a riding route, but the
/// cost stays a preference: when no conflict-free route exists, the least-
/// conflicting one is taken.
const SOFT_RIDE_COST: f32 = 100.0;
/// Additional routing cost when a segment rides *exactly collinearly* on a
/// peer's centerline (same coordinate, overlapping span) — the flat engine's
/// aligned dummy channels can produce this organically (two aligned chains
/// converge on one vertical line), and the invariant bars laying on top
/// outright, so this tier is what makes the separating jog win the cost
/// comparison (a 2-bend detour costs ~2×[`BEND_PENALTY`] plus a length
/// delta, far below the ~[`SOFT_RIDE_COST`] + this figure).
const SOFT_EXACT_RIDE_COST: f32 = 1000.0;
/// How many repair passes the global pass runs at most (M16). Each pass
/// rescans the routed set, rips up the edges involved in hard violations,
/// and reroutes them; a clean scan stops the loop early. Bounded, and
/// deterministic in outcome (the scan and the reroute order are fixed).
const MAX_RIPUP_PASSES: usize = 8;

/// The one obstacle world every router routes against (M11.5). One model
/// for every edge kind — direct, cross-boundary, forced, self-loop — and
/// one clearance function pair ([`Obstacles::seg_clear`] /
/// [`Obstacles::clear`]) that [`track_route`]'s internal check also uses,
/// eliminating the class of latent bug M11 hit once (`track_route`
/// clear-checking un-inflated rects while its caller inflated them).
///
/// Peer occupancy is split into a hard and a soft list (M16): hard segments
/// bar parallel riding absolutely (the old one-pass regime's backstop; the
/// global pass leaves it empty), soft segments are negotiable — riding one
/// costs [`SOFT_RIDE_COST`] but routes through it. Reserved port corridors
/// behave like hard bands: parallel riding inside one is barred (a port
/// must stay reachable), perpendicular crossing is legal.
#[derive(Clone)]
struct Obstacles {
    /// Node interiors (absolute rects). A route keeps [`ROUTE_PAD`] clear
    /// of them — except that it may touch one at a designated port: a
    /// segment end lying on the node's boundary, perpendicular to the edge
    /// it touches, with the rest of the segment staying outside the node
    /// (ports legitimately sit on node boundaries; a pass-through, or a
    /// leg sliding along an edge, is barred).
    nodes: Vec<(f32, f32, f32, f32)>,
    /// Hard peer-edge segments, each pre-inflated by
    /// [`SEGMENT_OBSTACLE_PAD`]. Only *collinear riding* is barred; a
    /// perpendicular crossing is legal — edges cross; the invariant bars
    /// laying on top, not crossing. The global routing pass (M16) keeps
    /// this list empty — peer segments are soft there — and it survives
    /// for callers that want the old absolute backstop (and for the unit
    /// fixtures).
    segments: Vec<(f32, f32, f32, f32)>,
    /// Soft peer-edge segments, each pre-inflated by
    /// [`SEGMENT_OBSTACLE_PAD`] (M16): negotiable occupancy. Riding one
    /// costs [`SOFT_RIDE_COST`] ([`Obstacles::soft_cost`]) but nothing is
    /// barred — the repair loop, not the current edge, resolves the
    /// conflict.
    soft: Vec<(f32, f32, f32, f32)>,
    /// Reserved approach corridors of *other* edges' endpoint node ports
    /// (M16): hard bands a route may not ride parallel inside (a port must
    /// stay reachable); perpendicular crossing is legal. An edge's own two
    /// corridors are never in its own world.
    corridors: Vec<(f32, f32, f32, f32)>,
    /// Thin bands centered on group frames' drawn outlines (absorbed
    /// M12). A route keeps only [`BORDER_PAD`] clear of a band, and may
    /// touch/cross one only at a designated crossing: a segment end lying
    /// on the outline, perpendicular to it. Riding along an outline
    /// (parallel) is always barred, and so is a transversal crossing
    /// without a recorded waypoint — crossing a frame is intentional and
    /// recorded ([`record_border_crossings`], and the stub builders'
    /// `line_rect_exit` clipping). A frame's interior stays legal: the
    /// bands, not the frame rect, are the obstacle.
    borders: Vec<(f32, f32, f32, f32)>,
    /// Ancestor-frame border bands (M20.2): the *cost* counterpart of
    /// `borders` for the whole-path search. A mid-run perpendicular
    /// transversal of one costs [`FRAME_CROSS_PENALTY`] in
    /// [`track_search`]'s Dijkstra — the strong preference for crossing
    /// each ancestor frame as few times as possible — while staying legal
    /// (a recorded waypoint clears it, as with `borders`). Nothing ever
    /// pushes into it outside the whole-path search, so the ordinary
    /// ladder's cost model (and every snapshot it re-derives) is
    /// untouched.
    cross_bands: Vec<(f32, f32, f32, f32)>,
    /// Group title text rects (M20.3): solid obstacles, so the whole-path
    /// search routes *around* a title instead of relying on the deleted
    /// title-detour geometry. A title sits inside its frame's top band, so
    /// this is the M20 obstacle model's title entry — the search discovers
    /// the detour the old hand-built one encoded.
    titles: Vec<(f32, f32, f32, f32)>,
    /// The whole-path search's port-stub freedom (M20.2). False for every
    /// other caller, so no other route's graph or cost model changes.
    port_graph: bool,
    /// The flat engine's own jog rows (flow midpoints of its waypoint pairs),
    /// seeded as extra tracks by the whole-path search only (M20.2). The
    /// assigned LCA lane joins them for a cross-boundary edge (M20.3), so
    /// the search's cross-run lands on the same row the ladder's Z used.
    jog_rows: Vec<f32>,
    /// Which axis the jog rows run across (M20.3): a vertical flow's jog rows
    /// are `y` values its cross-runs sit on, a horizontal flow's are `x`
    /// values. Only the jog-row preference term reads it.
    jog_axis: FlowAxis,
    /// The edge's *assigned* jog row (M20.3, cross-boundary edges only): the
    /// lane the M9 lane-separation pre-pass gave it. It carries a much
    /// stronger preference than the [`Obstacles::jog_rows`] hints
    /// ([`LANE_DRIFT_COST`]) because it is what keeps parallel edges through
    /// one gap apart — the search must land on it whenever the row is
    /// routable.
    lane_row: Option<f32>,
    /// The *assigned crossing coordinates* of the ancestor frame sides this
    /// edge traverses (M20.4): `(frame, side, cross)` triples from
    /// [`separate_frame_crossings`]. Each one is seeded as a track and given
    /// a [`CROSSING_DRIFT_COST`] preference, so the search lands its
    /// perpendicular border crossing on the fan coordinate the pre-pass
    /// allocated instead of on whichever row the heap reached first — the
    /// fan [`separate_ports`] gives a shared *node* side, extended to the
    /// frame sides the whole-path search crosses.
    crossing_rows: Vec<(usize, Side, f32)>,
}

impl Obstacles {
    fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            segments: Vec::new(),
            soft: Vec::new(),
            corridors: Vec::new(),
            borders: Vec::new(),
            cross_bands: Vec::new(),
            titles: Vec::new(),
            port_graph: false,
            jog_rows: Vec::new(),
            jog_axis: FlowAxis::Vertical,
            lane_row: None,
            crossing_rows: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.segments.is_empty()
            && self.soft.is_empty()
            && self.corridors.is_empty()
            && self.borders.is_empty()
            && self.cross_bands.is_empty()
            && self.titles.is_empty()
            && !self.port_graph
            && self.jog_rows.is_empty()
            && self.crossing_rows.is_empty()
    }

    fn push_node(&mut self, r: (f32, f32, f32, f32)) {
        self.nodes.push(r);
    }
    /// Push the ancestor-frame bands of one edge's frame world (M20.2):
    /// `cross_bands` for the whole-path search's crossing cost, and the
    /// same bands as `borders` — the ancestor band's parallel-riding bar
    /// and its designated-crossing waypoint rows are exactly the border
    /// semantics.
    fn push_cross_bands(&mut self, bands: &[(f32, f32, f32, f32)]) {
        self.cross_bands.extend_from_slice(bands);
        self.borders.extend_from_slice(bands);
    }

    /// Push a group title's text rect as a solid obstacle (M20.3).
    fn push_title(&mut self, r: (f32, f32, f32, f32)) {
        self.titles.push(r);
    }
    fn push_soft_segment(&mut self, s0: (f32, f32), s1: (f32, f32)) {
        self.soft.push(segment_rect(s0, s1, SEGMENT_OBSTACLE_PAD));
    }

    fn push_corridor(&mut self, r: (f32, f32, f32, f32)) {
        self.corridors.push(r);
    }

    /// The whole-path search's crossing cost (M20.2): a
    /// [`FRAME_CROSS_PENALTY`] per mid-segment perpendicular transversal
    /// of a `cross_bands` band. A band not in `cross_bands` costs nothing
    /// (the ordinary ladder's world never pushes into the list, so its
    /// cost model is untouched).
    fn cross_cost(&self, pts: &[(f32, f32)]) -> f32 {
        pts.windows(2)
            .map(|w| {
                self.cross_bands
                    .iter()
                    .filter(|&&b| band_crossing(w[0], w[1], b).is_some())
                    .count() as f32
                    * FRAME_CROSS_PENALTY
            })
            .sum()
    }

    /// The M20.4 re-crossing cost of a polyline: [`FRAME_RECROSS_COST`] per
    /// designated crossing of an ancestor band — every segment end that lies
    /// on a band's centerline and is perpendicular to it (the crossing
    /// waypoints [`record_border_crossings`] inserts, and the ports that sit
    /// on a band). Counted per crossing *event*, so a route that leaves and
    /// re-enters a frame pays twice while one straight descent pays once.
    fn recross_cost(&self, pts: &[(f32, f32)]) -> f32 {
        let mut n = 0usize;
        for w in pts.windows(2) {
            for &b in &self.cross_bands {
                if designated_crossing(w[0], w[1], b) {
                    n += 1;
                }
            }
        }
        n as f32 * FRAME_RECROSS_COST
    }

    /// The preference for *starting* a cross-run on one of the seeded jog
    /// rows (M20.3): [`JOG_DRIFT_COST`] per pixel of distance from the
    /// nearest jog row, plus [`LANE_DRIFT_COST`] per pixel from the edge's
    /// assigned lane ([`Obstacles::lane_row`]).
    ///
    /// Charged once per cross-run (at the bend that starts it), not per
    /// track hop: a run is split into one graph hop per intervening track, so
    /// a per-hop charge would scale with how many tracks happen to lie along
    /// the run rather than with the shape. A Z's total length is
    /// row-invariant, so without this term every row ties and the row is
    /// decided by heap order; with it the search lands on the row the
    /// deleted ladder's Z used (the flat engine's jog row, or the edge's
    /// assigned lane) — the row the M13 label anchors were tuned against.
    /// `dir` is the travel direction of the run being started (a
    /// [`Dir::code`]); only a cross-axis run has a row.
    fn row_drift_cost(&self, dir: usize, p: (f32, f32)) -> f32 {
        if self.jog_rows.is_empty() && self.lane_row.is_none() {
            return 0.0;
        }

        let cross_axis_dir = match self.jog_axis {
            FlowAxis::Vertical => dir == Dir::PosX.code() || dir == Dir::NegX.code(),
            FlowAxis::Horizontal => dir == Dir::PosY.code() || dir == Dir::NegY.code(),
        };
        if !cross_axis_dir {
            return 0.0;
        }
        let row = cross_flow(p, self.jog_axis).1;
        let mut cost = 0.0;
        if !self.jog_rows.is_empty() {
            let nearest = self
                .jog_rows
                .iter()
                .map(|&r| (row - r).abs())
                .fold(f32::INFINITY, f32::min);
            cost += JOG_DRIFT_COST * nearest;
        }
        if let Some(lane) = self.lane_row {
            cost += LANE_DRIFT_COST * (row - lane).abs();
        }
        cost
    }

    /// The preference for *starting* a flow-axis run on an assigned frame
    /// crossing (M20.4): [`CROSSING_DRIFT_COST`] per pixel of distance from
    /// the nearest crossing the separation pre-pass allocated to this edge
    /// ([`Obstacles::crossing_rows`]).
    ///
    /// A crossing of a frame's top/bottom border is a *flow*-axis run (the
    /// descent through the band), so its coordinate is an `x` for a
    /// left/right frame side — hence the pairing below (a Left/Right side's
    /// crossing constrains the cross-run's row; a Top/Bottom side's the
    /// flow-run's). Charged once per run start, exactly like
    /// [`Obstacles::row_drift_cost`], for the same reason: a run is many
    /// track hops but one shape choice.
    fn crossing_drift_cost(&self, dir: usize, p: (f32, f32)) -> f32 {
        if self.crossing_rows.is_empty() {
            return 0.0;
        }
        // A flow-axis run is the leg that travels *through* a Top/Bottom
        // frame border (it crosses the band), so it carries that crossing; a
        // cross-axis run is the leg that travels through a Left/Right border.
        let flow_run = match self.jog_axis {
            FlowAxis::Vertical => dir == Dir::PosY.code() || dir == Dir::NegY.code(),
            FlowAxis::Horizontal => dir == Dir::PosX.code() || dir == Dir::NegX.code(),
        };
        let want_side = |side: Side| match self.jog_axis {
            FlowAxis::Vertical => {
                if flow_run {
                    matches!(side, Side::Top | Side::Bottom)
                } else {
                    matches!(side, Side::Left | Side::Right)
                }
            }
            FlowAxis::Horizontal => {
                if flow_run {
                    matches!(side, Side::Left | Side::Right)
                } else {
                    matches!(side, Side::Top | Side::Bottom)
                }
            }
        };
        // The coordinate the run travels *along* (its offset from the frame
        // side): the cross coordinate of the point for a flow run, the flow
        // coordinate for a cross run — i.e. always the coordinate the
        // crossing coordinate is expressed in.
        let coord = if flow_run {
            cross_flow(p, self.jog_axis).0
        } else {
            cross_flow(p, self.jog_axis).1
        };
        let mut nearest = f32::INFINITY;
        for &(_, side, c) in &self.crossing_rows {
            if want_side(side) {
                nearest = nearest.min((coord - c).abs());
            }
        }

        if nearest.is_finite() {
            CROSSING_DRIFT_COST * nearest
        } else {
            0.0
        }
    }

    /// Is the polyline clear of the whole hard world (nodes, hard segments,
    /// corridors, borders)?
    fn clear(&self, pts: &[(f32, f32)]) -> bool {
        pts.windows(2).all(|w| self.seg_clear(w[0], w[1]))
    }

    /// Is one segment clear of the whole hard world? The single clearance
    /// rule every router (and [`track_route`]'s internal check) uses.
    fn seg_clear(&self, p0: (f32, f32), p1: (f32, f32)) -> bool {
        self.nodes.iter().all(|&r| seg_clear_solid(p0, p1, r))
            && self.titles.iter().all(|&r| seg_clear_title(p0, p1, r))
            && self.segments.iter().all(|&r| seg_clear_padded(p0, p1, r))
            && self.corridors.iter().all(|&r| seg_clear_padded(p0, p1, r))
            && self.borders.iter().all(|&b| seg_clear_border(p0, p1, b))
    }

    /// The routing cost of a polyline against the *soft* occupancy (M16):
    /// [`SOFT_RIDE_COST`] per soft band a segment rides parallel inside.
    /// Perpendicular crossings are free (edges cross by design); only
    /// laying on top of a peer is the negotiated conflict.
    fn soft_cost(&self, pts: &[(f32, f32)]) -> f32 {
        pts.windows(2).map(|w| self.soft_cost_seg(w[0], w[1])).sum()
    }

    /// The soft cost of one segment: [`SOFT_RIDE_COST`] per soft band it
    /// rides parallel inside, plus [`SOFT_EXACT_RIDE_COST`] more per band
    /// whose centerline it lays exactly on top of (the invariant-level
    /// defect the negotiation must always separate when it can).
    fn soft_cost_seg(&self, p0: (f32, f32), p1: (f32, f32)) -> f32 {
        self.soft
            .iter()
            .copied()
            .filter(|&r| !seg_clear_padded(p0, p1, r))
            .map(|r| {
                if rides_exactly(p0, p1, r) {
                    SOFT_RIDE_COST + SOFT_EXACT_RIDE_COST
                } else {
                    SOFT_RIDE_COST
                }
            })
            .sum()
    }
}

/// Does the axis-aligned segment `p0`→`p1` lay exactly on the centerline of
/// a peer-segment band `r` (parallel, essentially the same coordinate,
/// overlapping span) — i.e. the two edges would draw on top of each other?
fn rides_exactly(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    let vertical = (p1.0 - p0.0).abs() < 1e-6;
    if vertical != (r.3 > r.2) {
        return false; // orientations differ: perpendicular, legal crossing
    }
    if vertical {
        let cx = r.0 + r.2 / 2.0;
        if (p0.0 - cx).abs() > 1e-2 {
            return false;
        }
        let (l1, h1) = (p0.1.min(p1.1), p0.1.max(p1.1));
        let (l2, h2) = (r.1, r.1 + r.3);
        h1.min(h2) - l1.max(l2) > 1e-2
    } else {
        let cy = r.1 + r.3 / 2.0;
        if (p0.1 - cy).abs() > 1e-2 {
            return false;
        }
        let (l1, h1) = (p0.0.min(p1.0), p0.0.max(p1.0));
        let (l2, h2) = (r.0, r.0 + r.2);
        h1.min(h2) - l1.max(l2) > 1e-2
    }
}

/// The reserved approach pockets of one routed polyline (M16): one per
/// endpoint — the swept rect of the *actual* terminal approach segment
/// (first and last, truncated to [`PORT_RIGHT_OF_WAY`] and grown by
/// [`PORT_CORRIDOR_PAD`]). Derived from the route itself, not the nominal
/// side, so an edge whose real entry is perpendicular (an around-target
/// route, M14's decoupled bends) reserves the corridor it actually uses and
/// never blocks a sibling's straight approach through its unused nominal
/// port. Hard for peers (no parallel riding inside), invisible to the
/// edge's own route.
fn pocket_rects(pts: &[(f32, f32)]) -> Vec<(f32, f32, f32, f32)> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let sweep = |port: (f32, f32), prev: (f32, f32)| {
        // Truncate the approach segment from the port end to the right-of-way
        // length (it may be longer — a full descent — and only the escape
        // pocket needs reserving).
        let dx = prev.0 - port.0;
        let dy = prev.1 - port.1;
        let len = (dx.hypot(dy)).max(1e-6);
        let d = PORT_RIGHT_OF_WAY.min(len);
        let q = (port.0 + dx / len * d, port.1 + dy / len * d);
        segment_rect(port, q, PORT_CORRIDOR_PAD)
    };
    vec![
        sweep(pts[0], pts[1]),
        sweep(*pts.last().unwrap(), pts[pts.len() - 2]),
    ]
}

/// A route's total cost in one obstacle world (M20.3's repair-loop
/// keep-best rule): hard clearance first (a route that does not clear is
/// infinitely worse), then length + [`BEND_PENALTY`] per bend + soft peer
/// occupancy. Pure and deterministic.
fn route_score(pts: &[(f32, f32)], world: &Obstacles) -> f32 {
    if pts.len() < 2 {
        return f32::INFINITY;
    }
    if !world.clear(pts) {
        return f32::INFINITY;
    }
    let len: f32 = pts
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs())
        .sum();
    let bends = pts.windows(3).count() as f32 * BEND_PENALTY;
    len + bends + world.soft_cost(pts)
}

/// The hard violations of a routed set (M16's repair scan): the edge indices
/// that must move. Two checks, in the one obstacle world's vocabulary:
///
/// (a) a segment through a node interior or riding a frame outline — the
/// per-edge base world's own acceptance predicate (the terminal-fallback
/// defect); (b) two peers' segments riding collinearly within
/// [`SEGMENT_OBSTACLE_PAD`]. Who moves is attributed by the reserved port
/// pockets: a rider inside the *other* edge's pocket yields alone (the port
/// has right-of-way — the invader moves); a rider inside its *own* pocket
/// stays while the other yields; a mid-route ride away from any pocket is
/// mutual and both negotiate (the reroute splits them in index order).
fn scan_violations(
    paths: &[Option<Vec<(f32, f32)>>],
    ctxs: &[EdgeCtx],
    geom: &Geometry,
    pockets: &[Vec<(f32, f32, f32, f32)>],
) -> std::collections::BTreeSet<usize> {
    let mut out = std::collections::BTreeSet::new();
    // (a) hard-world defects.
    for (ei, path) in paths.iter().enumerate() {
        let Some(pts) = path else { continue };
        if !edge_world(geom, &ctxs[ei]).clear(pts) {
            out.insert(ei);
        }
    }
    // (b) collinear peer riding, attributed by the pockets.
    for (i, pi) in paths.iter().enumerate() {
        let Some(pi) = pi else { continue };
        for (j, pj) in paths.iter().enumerate().skip(i + 1) {
            let Some(pj) = pj else { continue };
            let mut verdict: Option<Vec<usize>> = None;
            'outer: for wa in pi.windows(2) {
                for wb in pj.windows(2) {
                    if !seg_clear_padded(
                        wa[0],
                        wa[1],
                        segment_rect(wb[0], wb[1], SEGMENT_OBSTACLE_PAD),
                    ) {
                        // j's leg rides on i's. The pockets say who yields.
                        let j_in_i = pockets[i]
                            .iter()
                            .any(|&c| !seg_clear_padded(wb[0], wb[1], c));
                        let i_in_j = pockets[j]
                            .iter()
                            .any(|&c| !seg_clear_padded(wa[0], wa[1], c));
                        verdict = Some(if j_in_i && !i_in_j {
                            vec![j]
                        } else if i_in_j && !j_in_i {
                            vec![i]
                        } else {
                            vec![i, j]
                        });
                        break 'outer;
                    }
                }
            }
            if let Some(v) = verdict {
                for &k in &v {
                    out.insert(k);
                }
            }
        }
    }
    out
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

/// Is the axis-aligned segment `p0`→`p1` clear of a group title's text rect
/// (M20.3)? Titles are solid: no port-touch exemption (a route never *ends*
/// on a title), and a small [`TITLE_OBSTACLE_PAD`] keeps the stroke off the
/// glyphs.
fn seg_clear_title(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
    segment_rect_interval(p0, p1, inflate(r, TITLE_OBSTACLE_PAD)).is_none()
}

/// The solid rect a group title reserves for routing (M20.3): its measured
/// text rect grown by [`TITLE_OBSTACLE_PAD`] and, on the right, by the
/// fallback margin a viewer's wider face can add ([`title_clear_gap`] minus
/// its base gap) — the guarantee the deleted title detour encoded, now
/// expressed as an obstacle so the search inherits it.
fn title_obstacle_rect(r: (f32, f32, f32, f32), title: &str) -> (f32, f32, f32, f32) {
    let w = text::measure(title, FRAME_TITLE_FONT_SIZE).width;
    let extra = title_clear_gap(w) - TITLE_CLEAR_GAP;
    (r.0, r.1, r.2 + extra, r.3)
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
/// apart where achievable), while a perpendicular crossing is legal — edges
/// cross; the invariant bars laying on top, not crossing. The pad (8 px)
/// stays below [`FAN_SEP`] (10 px) so legitimate parallel fan lanes still
/// clear.
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

/// Does the segment `p0`→`p1` make a *designated* crossing of band `b` — one
/// end on the band's centerline, perpendicular to the outline (exactly the
/// exemption [`seg_clear_border`] grants)? Used by the M20.4 re-crossing cost.
fn designated_crossing(p0: (f32, f32), p1: (f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let perpendicular = if b.3 <= b.2 {
        (p1.0 - p0.0).abs() < 1e-6
    } else {
        (p1.1 - p0.1).abs() < 1e-6
    };
    if !perpendicular {
        return false;
    }
    on_band_centerline(p0, b) || on_band_centerline(p1, b)
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
        if at_end && (on_band_centerline(p_touch, b) || in_band(p_touch, b)) {
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

/// Does `p` lie within the *thin band* `b` (its full thickness, not just the
/// centerline)? A track graph seeded with a crossing coordinate that is a
/// hair off a band's centerline (the M20.4 crossing fan allocates
/// arbitrary coordinates, so a track can sit inside a band's 1.5 px
/// thickness) would otherwise split one perpendicular crossing into two
/// hops, the second of which starts *beside* the centerline and so fails
/// [`seg_clear_border`]'s designated-crossing exemption. Treating a leg that
/// begins (or ends) anywhere inside the band as still "at" the crossing keeps
/// the exemption — a leg that stays inside the band and leaves it
/// perpendicularly is a designated crossing at that coordinate.
fn in_band(p: (f32, f32), b: (f32, f32, f32, f32)) -> bool {
    let (x, y, w, h) = b;
    if h <= w {
        p.1 >= y - BORDER_PAD - 1e-2
            && p.1 <= y + h + BORDER_PAD + 1e-2
            && p.0 >= x - 1e-2
            && p.0 <= x + w + 1e-2
    } else {
        p.0 >= x - BORDER_PAD - 1e-2
            && p.0 <= x + w + BORDER_PAD + 1e-2
            && p.1 >= y - 1e-2
            && p.1 <= y + h + 1e-2
    }
}

/// Insert waypoints where an axis-aligned segment crosses a frame band's
/// centerline transversally mid-segment, so the crossing is a designated
/// waypoint the clearance check exempts (M11.5: crossing a frame is
/// intentional and recorded — the within-frame stub builders already do
/// this via `line_rect_exit` clipping; the around-target route is the one
/// generator that still needs it). Crossings already marked by an endpoint
/// on the centerline are left alone.
fn record_border_crossings(pts: &[(f32, f32)], bands: &[(f32, f32, f32, f32)]) -> Vec<(f32, f32)> {
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

/// One of the four axis-aligned directions, as a unit step — the outward
/// normal of a port escape. The router uses it to bar legs that double back
/// over the edge's *own* escape stub (a first leg running toward the start
/// port, or a final leg running outward into the end escape point): the
/// obstacle world contains nodes and peers but never the edge's own stub,
/// so without this bar a reroute may ride the stub's line back toward the
/// port and reverse over it — an out-and-back spike on the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    PosX,
    NegX,
    PosY,
    NegY,
}

impl Dir {
    /// The outward normal of `side` (the direction [`escape_point`] moves
    /// along).
    fn of_side(side: Side) -> Dir {
        match side {
            Side::Top => Dir::NegY,
            Side::Bottom => Dir::PosY,
            Side::Left => Dir::NegX,
            Side::Right => Dir::PosX,
        }
    }

    fn opposite(self) -> Dir {
        match self {
            Dir::PosX => Dir::NegX,
            Dir::NegX => Dir::PosX,
            Dir::PosY => Dir::NegY,
            Dir::NegY => Dir::PosY,
        }
    }

    /// The Dijkstra direction code [`track_route`] uses (0=+x, 1=-x, 2=+y,
    /// 3=-y).
    fn code(self) -> usize {
        match self {
            Dir::PosX => 0,
            Dir::NegX => 1,
            Dir::PosY => 2,
            Dir::NegY => 3,
        }
    }
}

/// The outward escape directions of the port stubs attached around a routed
/// span (M11), when the caller escapes a port first. `None` on an end means
/// no stub is attached there (the route leg meets its continuation at a
/// plain corner). Constrains the ladder: a first leg may not run opposite
/// the start escape (back toward its port), and a final leg may not run
/// along the end escape's outward direction (into its escape point from
/// inside the stub span) — both ride the stub and reverse over it.
#[derive(Clone, Copy, Default)]
struct Escapes {
    start: Option<Dir>,
    end: Option<Dir>,
}

impl Escapes {
    /// Does a candidate polyline double back over an escape stub — its
    /// first leg running toward the start port, or its final leg running
    /// outward into the end escape point? A same-direction first leg is a
    /// harmless collinear extension of the stub; an inward final leg is the
    /// appended stub leg itself.
    fn defective(&self, pts: &[(f32, f32)]) -> bool {
        if pts.len() < 2 {
            return false;
        }
        let code = |p0: (f32, f32), p1: (f32, f32)| -> usize {
            if (p1.0 - p0.0).abs() >= (p1.1 - p0.1).abs() {
                if p1.0 > p0.0 { Dir::PosX } else { Dir::NegX }.code()
            } else if p1.1 > p0.1 {
                Dir::PosY.code()
            } else {
                Dir::NegY.code()
            }
        };
        if let Some(d) = self.start
            && code(pts[0], pts[1]) == d.opposite().code()
        {
            return true;
        }
        if let Some(d) = self.end {
            let n = pts.len();
            if code(pts[n - 2], pts[n - 1]) == d.code() {
                return true;
            }
        }
        false
    }
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
    /// M13: the edge label's knockout extent *across* the parallel runs this
    /// side carries ([`side_label_cover`]; 0 when unlabeled) — the fan gap
    /// grows with it (see [`label_pair_sep`]).
    cover: f32,
    /// Whether this end sits on a rep *frame's* border (the M16 frame
    /// rep-crossing fan) rather than on a node's side. Frame crossings host
    /// no label knockout (labels anchor at node ports and lane runs), so
    /// their fan gaps stay at plain [`FAN_SEP`].
    frame: bool,
    /// M17: the owning edge's endpoint ids — a content key for sorting, so
    /// tie-breaks never fall back to the declaration order (permuting the
    /// edges must permute nothing).
    from_id: String,
    to_id: String,
}

/// Assign fanned port cross-coordinates to node sides carrying more than one
/// edge. Returns a map from `(edge, is_from)` to the assigned cross; sides
/// with a single edge are absent (the caller keeps the node's centre).
///
/// Each edge's port is placed at its *other* endpoint's cross-coordinate
/// projected onto this side (clamped to the side, inset from the corners), so
/// an edge to a directly-aligned target stays straight; edges sharing a side
/// are then nudged apart to at least [`label_pair_sep`] — [`FAN_SEP`] for two
/// unlabeled edges, more when a label's knockout would reach the neighbor's
/// line (M13, capped at [`FAN_LABEL_CAP`]).
fn separate_ports(ends: &[PortEnd]) -> std::collections::HashMap<(usize, bool), f32> {
    use std::collections::HashMap;
    let mut groups: HashMap<(usize, Side), Vec<usize>> = HashMap::new();
    for (k, e) in ends.iter().enumerate() {
        groups.entry((e.node_idx, e.side)).or_default().push(k);
    }
    let mut out: HashMap<(usize, bool), f32> = HashMap::new();
    for ((_node_idx, side), idxs) in &groups {
        // M18: only ends that would actually ride matter — a fan entry for
        // ends that don't collide pins them apart for nothing.
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
        // that keeps fanned edges from crossing each other). Ties break on
        // the edge's endpoint ids (M17), never on declaration order.
        let mut order: Vec<usize> = idxs.clone();
        order.sort_by(|&a, &b| {
            ends[a]
                .other_cross
                .total_cmp(&ends[b].other_cross)
                .then_with(|| {
                    (
                        ends[a].from_id.as_str(),
                        ends[a].to_id.as_str(),
                        ends[a].is_from as u8,
                    )
                        .cmp(&(
                            ends[b].from_id.as_str(),
                            ends[b].to_id.as_str(),
                            ends[b].is_from as u8,
                        ))
                })
        });
        // Desired port = the other endpoint's cross, clamped to the side.
        let mut assigned: Vec<f32> = order
            .iter()
            .map(|&i| ends[i].other_cross.clamp(usable_lo, usable_hi))
            .collect();
        // The minimum gap between two adjacent ports is label-aware (M13): it
        // grows with the wider label's knockout reach across the runs. When
        // the required run overflows the usable span every gap shrinks
        // proportionally — deterministic best effort, and (unlike the old
        // push-and-clamp) never collapses two ports onto one coordinate (which
        // would stack their segments collinearly).
        let covers: Vec<f32> = order
            .iter()
            .map(|&i| if ends[i].frame { 0.0 } else { ends[i].cover })
            .collect();
        let mut seps: Vec<f32> = (0..covers.len().saturating_sub(1))
            .map(|k| label_pair_sep(covers[k], covers[k + 1], FAN_SEP, Some(FAN_LABEL_CAP)))
            .collect();
        let total: f32 = seps.iter().sum();
        let span = usable_hi - usable_lo;
        if total > span {
            let scale = span / total;
            for s in seps.iter_mut() {
                *s *= scale;
            }
        }
        // Enforce minimum fan separation left-to-right, then clamp, then a
        // right-to-left pass to restore separation if clamping bunched an end.
        for i in 1..assigned.len() {
            let s = seps[i - 1];
            if assigned[i] - assigned[i - 1] < s {
                assigned[i] = assigned[i - 1] + s;
            }
        }
        for v in assigned.iter_mut() {
            *v = v.clamp(usable_lo, usable_hi);
        }
        for i in (1..assigned.len()).rev() {
            let s = seps[i - 1];
            if assigned[i] - assigned[i - 1] < s {
                assigned[i - 1] = assigned[i] - s;
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

// ---- M20.4 — frame-side crossing separation ---------------------------
//
// The plan's "Pre-pass 1, reframed". Before M20.4 the port fan
// ([`separate_ports`]) spread the *node* ports of a shared side, but the
// whole-path search then chose where each edge crosses its ancestor frames'
// border bands on its own — and because every such crossing is a designated
// waypoint (a segment end on the band centerline, which [`seg_clear_border`]
// exempts), two edges entering the same frame through the same side could
// both pick the side's cheapest row and land on **one coordinate**. The
// result was a collinear overlap the repair loop then had to negotiate away
// by pushing the loser all the way around the frame.
//
// The cure is the M9 idea applied to the frames the search crosses: allocate
// each frame side's *crossing coordinates* up front, exactly as
// [`separate_ports`] allocates a node side's port coordinates, and seed them
// into the search's track graph as preferred crossing tracks
// ([`Obstacles::crossing_rows`]). The A* then returns crossings on (or near)
// the allocated coordinates — a fan instead of a collapse — and the repair
// loop has nothing to negotiate. The effect is measured by
// `frame_crossing_fan_seats_nested_frame_crossings` (which asserts phase 1
// equals the final routes: removing the fan costs 140.4 px of repair detour
// on that fixture, and 678.6 px on the `cluster` sample).

/// One edge end's traversal of one ancestor frame side: the crossing the
/// whole-path search will make of that frame's border band, gathered for the
/// separation pre-pass.
struct FrameCross {
    edge: usize,
    /// The group frame whose border is crossed.
    frame: usize,
    /// The frame side the edge passes through (the side facing the edge's
    /// travel — derived from which border band the edge's port-to-port span
    /// meets first).
    side: Side,
    /// The edge's own "desired" crossing coordinate: the cross-coordinate it
    /// would use if nothing else were crossing — the *inside* endpoint's
    /// column (see the `want` computation in [`assemble`]), which is the
    /// descent a lone edge would draw anyway.
    want: f32,
    /// M13: the edge label's knockout extent across the crossing runs
    /// ([`side_label_cover`]; 0 when unlabeled) — the fan gap grows with it.
    cover: f32,
    /// M17: the edge's endpoint ids — a content key for sorting, so ties
    /// never fall back to the declaration order.
    from_id: String,
    to_id: String,
}

/// Assign fanned crossing coordinates to frame sides carrying more than one
/// edge. Returns a map from `(edge, frame, side)` to the assigned crossing
/// coordinate; sides with a single crossing edge are absent (the search keeps
/// its own choice there, and for a lone edge that choice is already the
/// natural descent).
///
/// The algorithm is [`separate_ports`]'s, on the frame side's cross span
/// instead of a node's: order the crossings by their desired coordinate
/// (ties broken on the endpoint ids, never on declaration order), then nudge
/// them apart to at least [`label_pair_sep`] — [`FAN_SEP`] for two unlabeled
/// edges, more where a knockout would reach a neighbor's line — with the same
/// proportional shrink when the required run overflows the span and the same
/// clamp/restore passes. A side too narrow to fan (a frame thinner than the
/// ports it must host) yields nothing, and the search falls back to its own
/// choice.
fn separate_frame_crossings(
    crosses: &[FrameCross],
    frames: &std::collections::HashMap<usize, (f32, f32, f32, f32)>,
) -> std::collections::HashMap<(usize, usize, Side), f32> {
    use std::collections::HashMap;
    let mut groups: HashMap<(usize, Side), Vec<usize>> = HashMap::new();
    for (k, c) in crosses.iter().enumerate() {
        groups.entry((c.frame, c.side)).or_default().push(k);
    }
    let mut out: HashMap<(usize, usize, Side), f32> = HashMap::new();
    for ((frame, side), idxs) in &groups {
        if idxs.len() < 2 {
            continue;
        }
        let Some(&rect) = frames.get(frame) else {
            continue;
        };
        let (lo, hi) = side_cross_span(rect, *side);
        // The usable span: the frame's side, inset from its corners by
        // [`PORT_INSET`] (the same inset a node port fan uses — a crossing at
        // a frame corner would thread two border bands at once).
        let usable_lo = lo + PORT_INSET;
        let usable_hi = hi - PORT_INSET;
        if usable_hi <= usable_lo + 1e-3 {
            continue;
        }
        let mut order: Vec<usize> = idxs.clone();
        order.sort_by(|&a, &b| {
            crosses[a]
                .want
                .total_cmp(&crosses[b].want)
                .then_with(|| {
                    (
                        crosses[a].from_id.as_str(),
                        crosses[a].to_id.as_str(),
                    )
                        .cmp(&(crosses[b].from_id.as_str(), crosses[b].to_id.as_str()))
                })
        });
        let mut assigned: Vec<f32> = order
            .iter()
            .map(|&i| crosses[i].want.clamp(usable_lo, usable_hi))
            .collect();
        let covers: Vec<f32> = order.iter().map(|&i| crosses[i].cover).collect();
        let mut seps: Vec<f32> = (0..covers.len().saturating_sub(1))
            .map(|k| label_pair_sep(covers[k], covers[k + 1], FAN_SEP, Some(FAN_LABEL_CAP)))
            .collect();
        let total: f32 = seps.iter().sum();
        let span = usable_hi - usable_lo;
        if total > span {
            let scale = span / total;
            for s in seps.iter_mut() {
                *s *= scale;
            }
        }
        // The same three-pass spread [`separate_ports`] uses.
        for i in 1..assigned.len() {
            let s = seps[i - 1];
            if assigned[i] - assigned[i - 1] < s {
                assigned[i] = assigned[i - 1] + s;
            }
        }
        for v in assigned.iter_mut() {
            *v = v.clamp(usable_lo, usable_hi);
        }
        for i in (1..assigned.len()).rev() {
            let s = seps[i - 1];
            if assigned[i] - assigned[i - 1] < s {
                assigned[i - 1] = assigned[i] - s;
            }
        }
        for v in assigned.iter_mut() {
            *v = v.clamp(usable_lo, usable_hi);
        }
        for (k, &i) in order.iter().enumerate() {
            out.insert((crosses[i].edge, crosses[i].frame, crosses[i].side), assigned[k]);
        }
    }
    out
}

/// Is `p` inside rect `r` (inclusive of its boundary)?
fn rect_contains_point(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool {
    p.0 >= r.0 - 1e-2 && p.0 <= r.0 + r.2 + 1e-2 && p.1 >= r.1 - 1e-2 && p.1 <= r.1 + r.3 + 1e-2
}

/// The distance from `p` to the nearest point of rect `r` (0 inside it).
fn dist_to_rect(p: (f32, f32), r: (f32, f32, f32, f32)) -> f32 {
    let dx = (r.0 - p.0).max(0.0).max(p.0 - (r.0 + r.2));
    let dy = (r.1 - p.1).max(0.0).max(p.1 - (r.1 + r.3));
    dx.hypot(dy)
}

/// Which side of frame `frame` does the edge from `from` to `to` cross
/// *first*? The side the port-to-port segment meets on its way — the side
/// that faces the source when the source is outside, or the exit side when
/// the source is inside. `None` when the segment never meets the frame.
///
/// This is deliberately geometric rather than "the side facing the source
/// along the dominant axis": a wide frame (a whole row of groups) can have
/// its centre far from the edge's own column, so a centre comparison picks a
/// vertical side for an edge that plainly enters through the top.
fn frame_entry_side(
    frame: (f32, f32, f32, f32),
    from: (f32, f32),
    to: (f32, f32),
) -> Option<Side> {
    let (x, y, w, h) = frame;
    let (x1, y1) = (x + w, y + h);
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    // The parameter at which the segment meets each of the four side planes,
    // keeping only those whose meeting point lies within the side's span.
    let mut hits: Vec<(f32, Side)> = Vec::new();
    let mut try_hit = |t: f32, p: (f32, f32), side: Side| {
        if t > 1e-6 && t < 1.0 - 1e-6 && p.0 >= x - 1e-2 && p.0 <= x1 + 1e-2 && p.1 >= y - 1e-2
            && p.1 <= y1 + 1e-2
        {
            hits.push((t, side));
        }
    };
    if dy.abs() > 1e-6 {
        for (plane, side) in [(y, Side::Top), (y1, Side::Bottom)] {
            let t = (plane - from.1) / dy;
            let px = from.0 + t * dx;
            try_hit(t, (px, plane), side);
        }
    }
    if dx.abs() > 1e-6 {
        for (plane, side) in [(x, Side::Left), (x1, Side::Right)] {
            let t = (plane - from.0) / dx;
            let py = from.1 + t * dy;
            try_hit(t, (plane, py), side);
        }
    }
    // The earliest meeting point is the side the edge meets first.
    hits.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| (a.1 as u8).cmp(&(b.1 as u8))));
    hits.first().map(|&(_, side)| side)
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
    /// The (separated) target port's cross-coordinate, for lane ordering —
    /// with `from_cross` it gives the cross-interval each Z's vertical run
    /// spans (see [`lane_order`]).
    to_cross: f32,
    /// The flow axis of the edge's LCA level (obstacle band extents are
    /// read along it).
    axis: FlowAxis,
    /// The source's peer obstacles at the LCA level (everything except the
    /// source rep) — shared across a lane group, whose edges share a source
    /// and gap. Used to keep the lane band clear of obstacles that intrude
    /// into the gap.
    obstacles: Vec<(f32, f32, f32, f32)>,
    /// M13: the edge label's knockout extent *across* the lane's run
    /// ([`lane_label_cover`]; 0 when unlabeled) — the lane gap grows with it
    /// (see [`label_pair_sep`]).
    cover: f32,
    /// M17: the edge's endpoint ids — a content key for sorting, so
    /// tie-breaks never fall back to the declaration order (permuting the
    /// edges must permute nothing).
    from_id: String,
    to_id: String,
}

/// Assign distinct jog flow-coordinates (lanes) to parallel cross-boundary LCA
/// segments that share a source, source side, and target representative, so
/// they fan out in their shared gap instead of overlapping. Returns a map from
/// edge index to its jog flow-coordinate. A lone edge in a group gets the gap
/// midpoint (the prior behaviour).
///
/// The lanes spread across the usable band with a label-aware (M13) minimum
/// gap between adjacent lanes (see [`label_pair_sep`] / [`lane_label_cover`]):
/// the uniform spread whenever it already clears the labels, more where a
/// knockout would otherwise reach a neighbor's line. The lanes are ordered
/// along the band by [`lane_order`] — the source-port order adjusted so that
/// sibling Z-routes cannot cross each other.
///
/// One cross-boundary edge's gap span on its own side: the flow interval
/// between its two ports' flow-coordinates (low, high).
fn gap_span(edges: &[LaneEdge], k: usize) -> (f32, f32) {
    let e = &edges[k];
    (e.from_flow.min(e.to_flow), e.from_flow.max(e.to_flow))
}

fn assign_lanes(edges: &[LaneEdge]) -> std::collections::HashMap<usize, f32> {
    use std::collections::HashMap;
    // Group edges that share the gap (the flow span between the source's
    // exit and the target's entry) — these are the edges whose cross runs
    // land in the same band and can overlap. M9 grouped by source as well,
    // which left cross-source sharers of one gap uncoordinated: two edges
    // from different sources could park their jogs a few px apart (M16's
    // global pass then negotiates them apart by rip-up, at the cost of
    // whoever moves second's ideal shape). Grouping by gap alone spreads
    // every jog in one band together — the simultaneous-assignment reading
    // of lane separation. (Grouping by target alone would leave same-gap
    // edges like `api2 -> {db,cache,queue}` in separate single-edge
    // groups, all defaulting to the same midpoint.)
    //
    // The grouping is by gap-span *overlap* on a side, not by the exact
    // span: two edges whose gaps merely overlap (a wide gap like a
    // top-level source → deep target, and a narrow one inside it) jog in
    // the same shared band — an exact-span key would seat their lanes
    // independently and collide. Overlapping spans are swept into one
    // group (union by interval merge, per side); the group's jog band is
    // the intersection of its members' spans, shrunk away from the union
    // of the band obstacles below.
    // Sweep the edges per side by gap-span and merge overlapping spans into
    // one group (deterministic: sorted by (side, lo, hi), then endpoint ids).
    // M17 — union by interval merge: a group's membership test runs against
    // the running union (min lo, max hi) of its members' spans, not the head
    // edge's span, so chained overlaps (A[0,10], B[5,15], C[12,20]) land in
    // one group even though C misses the head. (M16 tested the head only,
    // which split such chains — and could seat a group whose members pairwise
    // overlap the head but not each other, an empty intersection.)
    let mut sweep: Vec<usize> = (0..edges.len()).collect();
    sweep.sort_by(|&a, &b| {
        edges[a].from_side.cmp(&edges[b].from_side).then_with(|| {
            let (la, ha) = gap_span(edges, a);
            let (lb, hb) = gap_span(edges, b);
            la.total_cmp(&lb)
                .then_with(|| ha.total_cmp(&hb))
                .then_with(|| {
                    (edges[a].from_id.as_str(), edges[a].to_id.as_str())
                        .cmp(&(edges[b].from_id.as_str(), edges[b].to_id.as_str()))
                })
        })
    });
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut unions: Vec<(f32, f32)> = Vec::new();
    for &k in &sweep {
        let (lo, hi) = gap_span(edges, k);
        let hit = groups.iter().zip(unions.iter()).position(|(g, &(m0, m1))| {
            edges[g[0]].from_side == edges[k].from_side && m0 < hi - 1e-6 && lo < m1 - 1e-6
        });
        match hit {
            Some(gi) => {
                groups[gi].push(k);
                unions[gi] = (unions[gi].0.min(lo), unions[gi].1.max(hi));
            }
            None => {
                groups.push(vec![k]);
                unions.push((lo, hi));
            }
        }
    }
    let mut out: HashMap<usize, f32> = HashMap::new();
    for idxs in &groups {
        let order = lane_order(edges, idxs);
        // The group's jog band: the intersection of its members' gap spans
        // (a lane must lie in every member's own gap to be usable by it).
        let gap_lo = idxs
            .iter()
            .map(|&k| gap_span(edges, k).0)
            .fold(f32::NEG_INFINITY, f32::max);
        let gap_hi = idxs
            .iter()
            .map(|&k| gap_span(edges, k).1)
            .fold(f32::INFINITY, f32::min);
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
        let mut band_obstacles: Vec<(f32, f32, f32, f32)> = Vec::new();
        for &i in &order {
            for &o in &edges[i].obstacles {
                if !band_obstacles.iter().any(|r| rects_near(*r, o)) {
                    band_obstacles.push(o);
                }
            }
        }
        for &o in &band_obstacles {
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
        if n > 1 && usable_hi > usable_lo {
            let band = usable_hi - usable_lo;
            // Label-aware lane gaps (M13): the gap between two adjacent lanes
            // is at least [`label_pair_sep`] (the wider knockout's reach plus
            // [`LABEL_CLEAR`], floored at [`FAN_SEP`]) and the band's slack is
            // shared out evenly on top — which reproduces the pre-M13 uniform
            // spread exactly for unlabeled groups while giving labeled gaps
            // their room first. When even the minimums overflow the band,
            // every gap shrinks proportionally — deterministic best effort;
            // the label placement engine's graze candidates resolve the rest.
            let mut seps: Vec<f32> = (0..n - 1)
                .map(|g| {
                    label_pair_sep(
                        edges[order[g]].cover,
                        edges[order[g + 1]].cover,
                        FAN_SEP,
                        None,
                    )
                })
                .collect();
            let total: f32 = seps.iter().sum();
            if total > band {
                let scale = band / total;
                for s in seps.iter_mut() {
                    *s *= scale;
                }
            } else {
                let slack = (band - total) / (n - 1) as f32;
                for s in seps.iter_mut() {
                    *s += slack;
                }
            }
            let used: f32 = seps.iter().sum();
            let mut lane = usable_lo + (band - used) / 2.0;
            for (k, &i) in order.iter().enumerate() {
                out.insert(edges[i].edge, lane);
                if k < seps.len() {
                    lane += seps[k];
                }
            }
        } else if gap_lo > gap_hi {
            // Empty intersection (M17's union merge can group chained
            // overlaps with no coordinate inside every member's own gap): no
            // common lane band exists, so each lane is clamped into its own
            // edge's gap — never outside the jogger's own gap, unlike the old
            // `gap_lo` default. The clamped fan below starts each lane at its
            // own gap's midpoint and pushes lanes apart left-to-right (and
            // back right-to-left where a clamp bunched an end), so members
            // with identical or nested gaps still get distinct jogs instead
            // of parking collinearly on one shared midpoint.
            let spans: Vec<(f32, f32)> = order
                .iter()
                .map(|&i| {
                    let (lo, hi) = gap_span(edges, i);
                    (lo + LANE_INSET, hi - LANE_INSET)
                })
                .collect();
            let mut assigned: Vec<f32> = spans
                .iter()
                .map(|&(lo, hi)| lo + (hi - lo).max(0.0) / 2.0)
                .collect();
            let seps: Vec<f32> = (0..n - 1)
                .map(|g| {
                    label_pair_sep(
                        edges[order[g]].cover,
                        edges[order[g + 1]].cover,
                        FAN_SEP,
                        None,
                    )
                })
                .collect();
            // Left-to-right push, clamp, right-to-left restore — the same
            // three passes [`separate_ports`] fans with.
            for k in 1..n {
                let s = seps[k - 1];
                if assigned[k] - assigned[k - 1] < s {
                    assigned[k] = assigned[k - 1] + s;
                }
            }
            for (k, v) in assigned.iter_mut().enumerate() {
                let (lo, hi) = spans[k];
                *v = v.clamp(lo.min(hi), hi.max(lo));
            }
            for k in (1..n).rev() {
                let s = seps[k - 1];
                if assigned[k] - assigned[k - 1] < s {
                    assigned[k - 1] = assigned[k] - s;
                }
            }
            for (k, v) in assigned.iter_mut().enumerate() {
                let (lo, hi) = spans[k];
                *v = v.clamp(lo.min(hi), hi.max(lo));
            }
            for (k, &i) in order.iter().enumerate() {
                out.insert(edges[i].edge, assigned[k]);
            }
        } else {
            // Single-member group (or a band pinched shut by obstacles): the
            // lone edge takes its own gap's midpoint — the pre-M17 default
            // for a single-edge group, restored here so a multi-group sweep
            // leaves every member inside its own gap band (a group-band
            // midpoint can sit outside a *narrow* member's own gap when the
            // union merge grouped it with wide neighbors).
            for &i in &order {
                let (lo, hi) = gap_span(edges, i);
                out.insert(edges[i].edge, lo + (hi - lo).max(0.0) / 2.0);
            }
        }
    }
    out
}

/// Deterministic non-crossing lane order for one lane group (see
/// [`assign_lanes`]): the base order is the source ports' cross-coordinates
/// (the fan order), adjusted so no two of the group's Z-routes cross.
///
/// With lanes increasing across the gap, an edge's Z spans the cross-
/// interval `V` between its two ports' cross-coordinates. For edges `a`,
/// `b` with `a`'s lane before `b`'s (closer to the gap's low end), `b`'s
/// source-side run reaches over `a`'s lane line when `b`'s source cross lies
/// in `V_a`, and `a`'s target-side run reaches under `b`'s lane line when
/// `a`'s target cross lies in `V_b` — either one crossing the other. (A
/// gap that runs right-to-left mirrors the two runs.) When exactly one of
/// the two orders is illegal the other becomes a constraint; when both are
/// — or neither — the base order stands. Constraints are resolved by
/// repeatedly taking the earliest (base-order) unconstrained edge (Kahn's
/// algorithm with a deterministic tie-break); a constraint cycle just falls
/// back to the base order for the remaining edges.
///
/// This is the shape behind "two sibling edges lie on top of each other":
/// when a fan edge's entry row falls between the source fan's rows (a target
/// nearly level with the source), the naive fan order forces its descent to
/// cross the next edge's source run and then run parallel beside it. The
/// reversed (non-crossing) order keeps the runs a full fan width apart.
fn lane_order(edges: &[LaneEdge], idxs: &[usize]) -> Vec<usize> {
    let mut base: Vec<usize> = idxs.to_vec();
    base.sort_by(|&a, &b| {
        edges[a]
            .from_cross
            .total_cmp(&edges[b].from_cross)
            // M18: break exact source-cross ties on the endpoint ids (the
            // M17 content key) — a fan's two edges can sit at the same
            // separated source cross, and a declaration-order tie would
            // permute the lane sweep (and the lanes with it) under the
            // permutation-invariance phase-1 test.
            .then_with(|| {
                (edges[a].from_id.as_str(), edges[a].to_id.as_str())
                    .cmp(&(edges[b].from_id.as_str(), edges[b].to_id.as_str()))
            })
    });
    let n = base.len();
    // May `base[a]`'s lane sit before `base[b]`'s without a crossing?
    let legal = |a: usize, b: usize| -> bool {
        let (fa, ta) = (edges[base[a]].from_cross, edges[base[a]].to_cross);
        let (fb, tb) = (edges[base[b]].from_cross, edges[base[b]].to_cross);
        let in_v = |x: f32, u: f32, v: f32| x > u.min(v) + 1e-6 && x < u.max(v) - 1e-6;
        if edges[base[a]].from_flow < edges[base[a]].to_flow {
            // Left-to-right gap: `b`'s source run spans up to its lane (over
            // `a`'s lane), `a`'s target run spans from its lane (under
            // `b`'s).
            !(in_v(fb, fa, ta) || in_v(ta, fb, tb))
        } else {
            // Right-to-left gap: the mirrored pair of runs.
            !(in_v(fa, fb, tb) || in_v(tb, fa, ta))
        }
    };
    // `pred[i]` = base indices that must be placed before `base[i]`.
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
    for a in 0..n {
        for b in (a + 1)..n {
            if !legal(a, b) && legal(b, a) {
                pred[a].push(b);
            } else if !legal(b, a) && legal(a, b) {
                pred[b].push(a);
            }
        }
    }
    let mut placed = vec![false; n];
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let pick = (0..n)
            .find(|&k| !placed[k] && pred[k].iter().all(|&p| placed[p]))
            .or_else(|| (0..n).find(|&k| !placed[k]))
            .expect("unplaced lane edge");
        placed[pick] = true;
        out.push(base[pick]);
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
    vec![a, with_flow(ca, lane, axis), with_flow(cb, lane, axis), b]
}

/// A rectilinear shortest path (fewest bends among shortest) from `a` to `b`
/// avoiding the obstacle world, routed along a track graph whose tracks are
/// the *local* obstacles' edges (node rects and frame bands) ±
/// [`TRACK_OFF`] plus the two ports' coordinates. Peer segments are not
/// tracked (they would sprawl the track graph over the whole canvas for
/// every edge): hard peer occupancy is clearance-checked like everything
/// else, and soft peer occupancy (M16) is charged in the Dijkstra cost
/// ([`Obstacles::soft_cost_seg`]) so routes prefer corridors clear of
/// parked peers but may cross negotiated space when nothing else is
/// available. When nothing blocks the only corridors, no route is found
/// and the caller falls back (the bounded ladder). Used when the simple Z
/// is blocked; returns the polyline `a -> ... -> b`, or an empty vec if no
/// route exists (caller falls back to the simple Z). `esc` bars the
/// double-back-over-own-escape-stub legs when the span starts/ends at an
/// escaped port (see [`Escapes`]).
fn track_route(a: (f32, f32), b: (f32, f32), obs: &Obstacles, esc: Escapes) -> Vec<(f32, f32)> {
    // Vertical tracks (x) and horizontal tracks (y), seeded from node rects
    // and frame bands. A frame band also seeds its centerline (the drawn
    // outline) itself: stopping on the centerline is a designated crossing,
    // so a route can cross a border legally in two perpendicular hops
    // (out to the outline, then past it) — without it, a band seals the
    // whole region it spans and no route can ever traverse a frame except
    // at the caller's own crossing waypoints.
    let mut vts = vec![a.0, b.0];
    let mut hts = vec![a.1, b.1];
    for &(x, y, w, h) in obs.nodes.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
    }
    for &(x, y, w, h) in obs.titles.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
    }
    for &(x, y, w, h) in obs.borders.iter() {
        vts.push(x - TRACK_OFF);
        vts.push(x + w + TRACK_OFF);
        hts.push(y - TRACK_OFF);
        hts.push(y + h + TRACK_OFF);
        if h <= w {
            hts.push(y + h / 2.0);
        } else {
            vts.push(x + w / 2.0);
        }
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
    let (Some(ai), Some(aj), Some(bi), Some(bj)) = (
        vts.iter().position(|&x| (x - a.0).abs() < 1e-2),
        hts.iter().position(|&y| (y - a.1).abs() < 1e-2),
        vts.iter().position(|&x| (x - b.0).abs() < 1e-2),
        hts.iter().position(|&y| (y - b.1).abs() < 1e-2),
    ) else {
        return Vec::new();
    };
    let pts = track_search(
        (ai, aj),
        (bi, bj),
        &vts,
        &hts,
        obs,
        esc,
        (
            minx - TRACK_OFF,
            miny - TRACK_OFF,
            maxx + TRACK_OFF,
            maxy + TRACK_OFF,
        ),
    );
    // Re-record the border crossings the collapse may have folded away: a
    // straight run hopping across a band's centerline collapses to one
    // segment through the band, which the clearance check would bar —
    // re-inserting the crossing waypoints keeps it a designated crossing.
    if !pts.is_empty() && !obs.borders.is_empty() {
        record_border_crossings(&pts, &obs.borders)
    } else {
        pts
    }
}

/// The track-graph Dijkstra shared by [`track_route`] (its own derived
/// region) and [`track_route_in_region`] (a caller-chosen region — M20.1's
/// whole-path groundwork): the graph (sorted, deduped tracks), the start
/// and goal intersections, and the obstacle world are given; the search is
/// the same bend-penalised, soft-costing Dijkstra with the M11 escape bars.
/// `bounds` is the region's `(minx, miny, maxx, maxy)` — the world's
/// extent in [`track_route`], the widened region in
/// [`track_route_in_region`] — used only for the frame-crossing cost
/// lookahead below (no cost term depends on it today; the parameter is the
/// seam the two callers share).
fn track_search(
    start_ij: (usize, usize),
    goal_ij: (usize, usize),
    vts: &[f32],
    hts: &[f32],
    obs: &Obstacles,
    esc: Escapes,
    _bounds: (f32, f32, f32, f32),
) -> Vec<(f32, f32)> {
    let nv = vts.len();
    let nh = hts.len();
    let (ai, aj) = start_ij;
    let (bi, bj) = goal_ij;
    let start = ai * nh + aj;
    let goal = bi * nh + bj;

    // Clear check for a segment between two track intersections — the same
    // single rule every router uses ([`Obstacles::seg_clear`], with its
    // per-kind inflations and designated-crossing exemptions), so a route
    // found here is one its caller's own check accepts. Soft peer occupancy
    // (M16) does not block a track — it is charged in the cost below, so a
    // Dijkstra route prefers corridors clear of parked peers but may cross
    // negotiated space when nothing else is available.
    let clear = |x0: f32, y0: f32, x1: f32, y1: f32| -> bool { obs.seg_clear((x0, y0), (x1, y1)) };
    // Neighbours of node (i,j): the adjacent track intersections reachable by
    // a clear orthogonal segment, with the direction of travel (for bend cost)
    // and the segment's soft-occupancy cost (M16).
    // dir: 0=+x,1=-x,2=+y,3=-y.
    let neighbours = |i: usize, j: usize| -> Vec<(usize, usize, usize, f32, f32)> {
        let mut out = Vec::new();
        let x = vts[i];
        let y = hts[j];
        let cost = |x1: f32, y1: f32| {
            let seg = [(x, y), (x1, y1)];
            obs.soft_cost_seg((x, y), (x1, y1))
                + obs.cross_cost(&seg)
                + obs.recross_cost(&seg)
        };
        if j + 1 < nh && clear(x, y, x, hts[j + 1]) {
            out.push((i, j + 1, 2, (hts[j + 1] - y).abs(), cost(x, hts[j + 1])));
        }
        if j > 0 && clear(x, y, x, hts[j - 1]) {
            out.push((i, j - 1, 3, (y - hts[j - 1]).abs(), cost(x, hts[j - 1])));
        }
        if i + 1 < nv && clear(x, y, vts[i + 1], y) {
            out.push((i + 1, j, 0, (vts[i + 1] - x).abs(), cost(vts[i + 1], y)));
        }
        if i > 0 && clear(x, y, vts[i - 1], y) {
            out.push((i - 1, j, 1, (x - vts[i - 1]).abs(), cost(vts[i - 1], y)));
        }
        if obs.port_graph {
            // The ports are points, not tracks: let the start leave for, and
            // the goal be entered from, any track visible along the shared
            // row/column — so the approach stub's length is a real choice.
            let mut long = |ti: usize, tj: usize| {
                if (ti, tj) == (i, j) {
                    return;
                }
                let (tx, ty) = (vts[ti], hts[tj]);
                if (tx - x).abs() > 1e-6 && (ty - y).abs() > 1e-6 {
                    return;
                }
                if !clear(x, y, tx, ty) {
                    return;
                }
                let len = (tx - x).abs() + (ty - y).abs();
                let dir = if (tx - x).abs() > 1e-6 {
                    if tx > x { 0 } else { 1 }
                } else if ty > y {
                    2
                } else {
                    3
                };
                out.push((ti, tj, dir, len, cost(tx, ty)));
            };
            if i == ai {
                for tj in 0..nh {
                    long(ai, tj);
                }
            }
            if j == aj {
                for ti in 0..nv {
                    long(ti, aj);
                }
            }
            if i == bi {
                long(bi, bj);
            }
            if j == bj {
                long(bi, bj);
            }
        }
        out
    };

    // Dijkstra over (node, incoming_dir) states so a bend (direction change)
    // can be penalised. 4 dirs; the start has no incoming dir (use 4 = none).
    // A binary heap with lazy deletion keeps the work proportional to the
    // graph size (the state count grows with the world's obstacle count).
    // Escape bars (M11): the first leg out of `a` may not run back over the
    // start stub, and the last leg into `b` may not run outward along the
    // end stub — both would draw an out-and-back spike over the edge's own
    // escape segment.
    let bar_first = esc.start.map(|d| d.opposite().code());
    let bar_last = esc.end.map(|d| d.code());
    let n_states = nv * nh * 5;
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
        for (ni, nj, d_out, len, soft) in neighbours(i, j) {
            if node == start && Some(d_out) == bar_first {
                continue;
            }
            if ni * nh + nj == goal && Some(d_out) == bar_last {
                continue;
            }
            let v = ni * nh + nj;
            // Both terminal legs get the stub preference: the goal's because
            // the arrowhead's back-extent must fit under it, the source's
            // because a cross-run jammed against its port reads as a kink in
            // the box rather than an edge. (Without the source term a jog row
            // a few pixels from the port wins on the lane preference alone.)
            let terminal = obs.port_graph && (node == start || v == goal);
            let cost = len
                + soft
                + if d_in != d_out {
                    // Starting a run: charge the row preference once, at the
                    // bend (a run is many track hops but one shape choice).
                    obs.row_drift_cost(d_out, (vts[ni], hts[nj]))
                        + obs.crossing_drift_cost(d_out, (vts[ni], hts[nj]))
                } else {
                    0.0
                }
                + if d_in != 4 && d_in != d_out {
                    BEND_PENALTY
                } else {
                    0.0
                }
                + if terminal {
                    STUB_SHORTFALL_COST * (MIN_TERMINAL_STUB - len).max(0.0)
                } else {
                    0.0
                };
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
    // Ensure the endpoints are exactly the start/goal intersections (snap
    // away float drift). [`track_route`] re-snaps to the exact port points
    // afterwards.
    if !pts.is_empty() {
        pts[0] = (vts[ai], hts[aj]);
        *pts.last_mut().unwrap() = (vts[bi], hts[bj]);
    }
    pts
}

/// [`track_route`] with one bounded negotiation retry (M18): when the
/// routed polyline still rides parked peer bands ([`Obstacles::soft`]), the
/// soft occupancy is meant as negotiable space — but a bare Dijkstra over
/// the same graph can return the same ridden corridor when the ridden
/// shape is otherwise best. The retry pins every soft band the first route
/// rides as a hard segment and routes again: if a corridor exists that is
/// clear of the *whole* world (including the pinned bands) and strictly
/// less soft-conflicting, it moves; otherwise the ridden route stands (the
/// bounded-ladder convention). Phase-1 routing has no peers (empty soft),
/// so the retry only ever fires in the phase-2 repair loop — the routing
/// result stays simultaneous-assignment-clean.
fn track_route_negotiated(
    a: (f32, f32),
    b: (f32, f32),
    obs: &Obstacles,
    esc: Escapes,
) -> Vec<(f32, f32)> {
    let first = track_route(a, b, obs, esc);
    if first.is_empty() || obs.soft_cost(&first) <= 1e-6 {
        return first;
    }
    let mut around = obs.clone();
    let mut pinned = false;
    for w in first.windows(2) {
        for &r in &obs.soft {
            // Pin only the *exact* rides (same centerline, overlapping
            // span) — the invariant-level defect this retry exists to
            // clear. Near-but-offset rides are the ordinary soft cost the
            // first route already paid; pinning those as well can seal the
            // only corridor instead of moving the exact rider.
            if rides_exactly(w[0], w[1], r) {
                // Push the pinned band as a *border* rather than a bare
                // segment: the border semantics bar the parallel ride on
                // the centerline (why we pin) AND seed its edges as track
                // rows, so the retry can fan onto a neighboring corridor
                // row instead of finding no route at all.
                around.borders.push(r);
                pinned = true;
            }
        }
    }
    if !pinned {
        return first;
    }
    let second = track_route(a, b, &around, esc);
    if !second.is_empty()
        && obs.clear(&second)
        && obs.soft_cost(&second) < obs.soft_cost(&first) - 1e-6
    {
        second
    } else {
        first
    }
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
    esc: Escapes,
) -> Vec<(f32, f32)> {
    if obs.is_empty() {
        return simple_z(a, b, axis, lane);
    }
    // The ladder, negotiated (M16): each rung is hard-clearance-checked;
    // among the hard-clear rungs the least soft-conflicting wins (earliest
    // rung on ties). A parked peer's soft band therefore steers the route
    // away from it only when a clean shape exists at some rung — soft
    // occupancy never blocks the ladder outright. Rungs that double back
    // over the span's own escape stubs are not candidates at all (M11):
    // the obstacle world never contains the edge's own stub, so without
    // this an otherwise-cheap rung could ride the stub's line back toward
    // the port and reverse over it.
    let mut best: Option<(Vec<(f32, f32)>, f32)> = None;
    fn consider(obs: &Obstacles, best: &mut Option<(Vec<(f32, f32)>, f32)>, pts: Vec<(f32, f32)>) {
        if obs.clear(&pts) {
            let c = obs.soft_cost(&pts);
            if best.as_ref().is_none_or(|(_, bc)| c < bc - 1e-6) {
                *best = Some((pts, c));
            }
        }
    }
    let z = simple_z(a, b, axis, lane);
    if !esc.defective(&z) {
        consider(obs, &mut best, z.clone());
        if best.as_ref().is_some_and(|(_, c)| *c < 1e-6) {
            return z;
        }
    }
    let mid = jog_flow(cross_flow(a, axis).1, cross_flow(b, axis).1);
    let zm = simple_z(a, b, axis, mid);
    if !esc.defective(&zm) {
        consider(obs, &mut best, zm.clone());
        if best.as_ref().is_some_and(|(_, c)| *c < 1e-6) {
            return best.expect("clean rung").0;
        }
    }
    let tr = track_route_negotiated(a, b, obs, esc);
    if !tr.is_empty() {
        consider(obs, &mut best, tr);
    }
    match best {
        Some((pts, _)) => pts,
        // Last resort: the simple Z at the lane (may cross, but is shortest)
        // — or, when that would spike over an escape stub, the midpoint Z if
        // it would not. Something must be returned; this is the bounded
        // fallback.
        None => {
            if esc.defective(&z) && !esc.defective(&zm) {
                zm
            } else {
                z
            }
        }
    }
}

// ---- M11 — forced edge sides (`from=` / `to=`) ----
//
// An edge may request the page-space side of each node it connects to.
// The request binds the endpoint **node** only (M14): the frames an edge
// crosses are the search's business (M20.3), so a forced side never drags a
// frame crossing with it. It is honored **literally**: the port lands
// on the requested side, with no automatic reversion to the algorithm's
// choice (silently second-guessing it would leave the user unable to tell
// whether it did anything). A side that contradicts the approach direction is routed
// around — the result may be ugly, and that is the user's cue to change or
// drop the attribute — but it stays orthogonal, never passes through a node
// interior (the `route_lca` ladder's bounded last resort excepted), and
// ports sharing a forced side are still fanned apart (M9 port separation).

/// The terminal fallback for a *forced* cross-boundary edge (M20.3): the
/// same escape-and-ladder shape [`forced_direct_path`] gives a forced direct
/// edge — each node port escapes outward along its requested side by
/// [`SIDE_ESCAPE`], then the [`route_lca`] ladder routes between the escaped
/// points against the M20 world. The rep model is gone, so there is no
/// frame-crossing decomposition left to honor: the frame crossings are the
/// ladder's business (its `borders` are the edge's ancestor bands), and the
/// only guarantee this route owes the user is the literal port placement
/// plus orthogonality.
fn forced_cross_boundary_path(ctx: &EdgeCtx, world: &Obstacles) -> Vec<(f32, f32)> {
    forced_direct_path(
        ctx.from_port,
        ctx.from_side,
        ctx.to_port,
        ctx.to_side,
        ctx.flow,
        world,
    )
}

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
    pts.extend(route_lca(
        a,
        b,
        axis,
        lane,
        obs,
        Escapes {
            start: Some(Dir::of_side(from_side)),
            end: Some(Dir::of_side(to_side)),
        },
    ));
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
        return vec![
            p1,
            escape_point(p1, from_side, out),
            escape_point(p2, to_side, out),
            p2,
        ];
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
        (
            if bot_room > top_room {
                y + h + out
            } else {
                y - out
            },
            false,
        )
    } else {
        // Top/Bottom pair: the corner line is vertical (wrap around the
        // left or the right side).
        let left_room = x;
        let right_room = canvas_w - (x + w);
        (
            if right_room > left_room {
                x + w + out
            } else {
                x - out
            },
            true,
        )
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
fn segment_rect(s0: (f32, f32), s1: (f32, f32), t: f32) -> (f32, f32, f32, f32) {
    (
        s0.0.min(s1.0) - t,
        s0.1.min(s1.1) - t,
        (s1.0 - s0.0).abs() + 2.0 * t,
        (s1.1 - s0.1).abs() + 2.0 * t,
    )
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
    pack_components(&mut nodes, &dag_edges);

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
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
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

// ============ Phase 2b: component packing ============

/// Re-base ranks so every weakly connected component of the DAG occupies its
/// own disjoint run of ranks, stacked along the rank axis in declaration
/// order (of each component's first-declared member).
///
/// Longest-path ranking only constrains nodes that participate in an edge;
/// edge-less nodes keep rank 0 and would otherwise all crowd into the
/// source band of the whole scope — a single row (top-down) or column
/// (left-right) running *perpendicular* to the requested flow axis.
/// Packing gives each component — including rank-0 singletons — its own
/// band range, so every node follows its scope's direction, edge or no
/// edge (the rank axis is the flow axis after the direction transform).
///
/// Runs before dummy insertion: dummies, crossing minimization, coordinate
/// assignment and band stacking all operate on these (now packed) global
/// ranks unchanged. Within a component, relative ranks — and therefore the
/// internal arrangement — are preserved (a uniform shift). Consecutive
/// components are separated by exactly one inter-rank gap, like
/// consecutive ranks.
fn pack_components(nodes: &mut [LNode], dag_edges: &[(usize, usize)]) {
    let n = nodes.len();
    if n <= 1 {
        return; // one node is its own component at rank 0 — nothing to pack
    }
    // Union-find over nodes joined by DAG edges (self-loops excluded: they
    // don't constrain rank). Roots are unioned toward the smaller index, so
    // a component's root is its first-declared member.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]]; // path halving
            x = parent[x];
        }
        x
    }
    for &(u, v) in dag_edges {
        if u == v {
            continue;
        }
        let (ru, rv) = (find(&mut parent, u), find(&mut parent, v));
        if ru != rv {
            let (lo, hi) = (ru.min(rv), ru.max(rv));
            parent[hi] = lo;
        }
    }
    // Members per component, keyed by root (empty slots are non-roots).
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        members[find(&mut parent, i)].push(i);
    }
    // Walk components in root order (= declaration order of their first
    // member) and shift each onto the next free rank range.
    let mut base = 0usize;
    for (root, comp) in members.iter().enumerate() {
        if comp.is_empty() {
            continue; // non-root index (or lone node in an empty scope)
        }
        debug_assert_eq!(root, find(&mut parent, root));
        // Height of the component in ranks; its lowest rank is 0 by
        // longest-path (every component has an in-component source).
        let height = comp.iter().map(|&i| nodes[i].rank).max().unwrap_or(0);
        for &i in comp {
            nodes[i].rank += base;
        }
        base += height + 1;
    }
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
    let mut keyed: Vec<(usize, f32, usize)> = Vec::with_capacity(layer.len());
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
        a.1.partial_cmp(&b.1)
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
    // below only translates each rank as a whole. Zero-size dummies carry
    // no width and *share a position* with their left neighbor (M18: a
    // left-pack step keeps them at the previous item's center, so the two
    // chains sharing a dummy rank stay distinct — previously the step was
    // `w + NODE_SEP`, which advanced by the *empty* step and let two
    // dummies land on the same packed slot when the item widths above were
    // zero too).
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
        // Source ranks (no parents) center on 0.0 — the left-pack origin —
        // so every component's block is centered rather than left-flush.
        let anchor = if parents.is_empty() {
            0.0
        } else {
            parents.iter().map(|&u| x[u] + w[u] / 2.0).sum::<f32>() / parents.len() as f32
        };

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
        let band = layer.iter().map(|&v| h[v]).fold(0.0_f32, f32::max);
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
    let max_right = (0..n).map(|i| nx[i] + w[i]).fold(0.0_f32, f32::max);
    let max_bottom = (0..n).map(|i| ny[i] + h[i]).fold(0.0_f32, f32::max);
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

// ============ M13 — label placement engine + parallel-edge spacing ============
//
// Edge labels used to be placed blind at the midpoint of their polyline's
// longest segment — the M9 known limitation: in a crowded gap one label's
// white knockout covered a neighboring edge's line or another label's text.
// M13 moves the anchor decision into the layout: `assemble` computes each
// label's anchor where the complete routed geometry is known and stores it on
// the `EdgePath` (a per-edge field, so the index-correspondence invariant is
// safe); the renderer just draws the knockout rect + text at the anchor.
//
// The placement engine enumerates candidate anchors along the polyline
// (offsets along the segments — centered on the line, plus the two
// perpendicular "graze" variants whose knockout edge still crosses the line
// while the bulk of the rect clears one side), scores each against what it
// must dodge — other edges' segments, other labels' knockouts, node rects,
// rendered arrowheads, and group titles (deliberately *not* group frame
// borders, M13.1: a label may cross a frame outline) — and takes the deterministic minimum: a greedy
// pass in declaration order, then one refinement pass in the same fixed order
// in which each label re-optimizes against every other label's current rect
// and moves only on a strict improvement. No randomness anywhere.
//
// The spacing half makes the M9 fan/lane separation label-aware: the gap
// between two adjacent fan ports or jog lanes grows with the labels' knockout
// sizes ([`label_pair_sep`] + [`side_label_cover`] / [`lane_label_cover`]), so
// an on-line knockout cannot reach a neighbor's line wherever the room
// exists; the graze candidates resolve what remains.

/// The knockout rect size `(width, height)` of an edge label: the measured
/// text block plus [`LABEL_PAD`] each side — exactly the rect the renderer
/// draws around the anchor [`place_labels`] picks. This is the single
/// definition both sides use (the renderer re-derives it from the same two
/// constants); keep the formulas together.
fn label_box(label: &str) -> (f32, f32) {
    let m = text::measure(label, EDGE_LABEL_SIZE);
    (m.width + 2.0 * LABEL_PAD, m.height + 2.0 * LABEL_PAD)
}

/// The knockout rect (top-left `(x, y, w, h)` form) of a label of knockout
/// `size` centered at `anchor`.
fn label_rect(anchor: (f32, f32), size: (f32, f32)) -> (f32, f32, f32, f32) {
    (
        anchor.0 - size.0 / 2.0,
        anchor.1 - size.1 / 2.0,
        size.0,
        size.1,
    )
}

/// The minimum gap between two adjacent parallel runs where one of their
/// labels may sit (M13): at least `base` ([`FAN_SEP`] for fan ports, the
/// uniform lane spread for lanes), and enough that neither on-line knockout
/// reaches the other's line — half the wider label's cross-run knockout
/// extent (`cover_a` / `cover_b` from [`side_label_cover`] /
/// [`lane_label_cover`]; 0 for an unlabeled edge) plus [`LABEL_CLEAR`].
/// `cap` bounds the label-aware bump where the runs cannot spread further
/// (fan ports on a real node side — see [`FAN_LABEL_CAP`]; lanes need no cap,
/// their gap band clamps them).
fn label_pair_sep(cover_a: f32, cover_b: f32, base: f32, cap: Option<f32>) -> f32 {
    let cover = cover_a.max(cover_b);
    if cover <= 0.0 {
        return base;
    }
    let mut need = cover / 2.0 + LABEL_CLEAR;
    if let Some(c) = cap {
        need = need.min(c);
    }
    base.max(need)
}

/// A label's knockout extent *across* the runs leaving a node `side` (M13):
/// the runs follow the side's normal, so the covering reach is the knockout's
/// width for a Top/Bottom side (vertical runs) and its height for a Left/Right
/// side (horizontal runs). 0 for an unlabeled edge.
fn side_label_cover(side: Side, size: Option<(f32, f32)>) -> f32 {
    match (side, size) {
        (Side::Top | Side::Bottom, Some((w, _))) => w,
        (Side::Left | Side::Right, Some((_, h))) => h,
        (_, None) => 0.0,
    }
}

/// A label's knockout extent *across* a jog lane's run (M13): the run follows
/// the cross axis, so the covering reach is the knockout's flow-axis extent —
/// its height for a vertical flow (horizontal runs), its width for a
/// horizontal flow (vertical runs). 0 for an unlabeled edge.
fn lane_label_cover(axis: FlowAxis, size: Option<(f32, f32)>) -> f32 {
    match (axis, size) {
        (FlowAxis::Vertical, Some((_, h))) => h,
        (FlowAxis::Horizontal, Some((w, _))) => w,
        (_, None) => 0.0,
    }
}

/// The rect a group's rendered title text occupies (M13 — the label
/// placement engine keeps knockouts off it): `x ∈ [frame.x + FRAME_TITLE_X,
/// … + measured width]`, `y ∈ [frame.y + FRAME_TITLE_TOP, … + lines *
/// line_height]` — matching the renderer's hanging-baseline title block.
fn title_text_rect(frame: (f32, f32, f32, f32), title: &str) -> (f32, f32, f32, f32) {
    let n = text::line_count(title);
    let lh = text::line_height(FRAME_TITLE_FONT_SIZE);
    let w = text::measure(title, FRAME_TITLE_FONT_SIZE).width;
    // M20.3: the title's obstacle rect is the drawn glyph box *plus* the
    // title band's remaining height, so the clearance the deleted title
    // detour encoded (the stub jogged in the band below the text) is
    // preserved: a within-frame descent may not thread the narrow strip
    // between the title's baseline and the first rank either, because the
    // search has no way to know that strip is meant to stay clear.
    (
        frame.0 + FRAME_TITLE_X,
        frame.1 + FRAME_TITLE_TOP,
        w,
        n as f32 * lh,
    )
}

/// Candidate tap fractions along a segment, in preference order — the segment
/// midpoint (the classic longest-segment look) first.
const LABEL_TAPS: [f32; 5] = [0.5, 1.0 / 3.0, 2.0 / 3.0, 0.25, 0.75];
/// How much of its own line a graze (offset) label's knockout still covers:
/// the graze slides the rect perpendicular to the line until only this much
/// of its half-extent hangs over — the line still reads as broken behind the
/// text while the bulk of the knockout clears one side.
const LABEL_ON_LINE: f32 = 2.5;
/// A peer edge line this close to the knockout edge still counts as covered:
/// the drawn stroke is 1.5 px wide, centered on the centerline the clearance
/// math uses.
const LABEL_HIT_PAD: f32 = 1.0;
// Scoring weights — deterministic, and the collision terms dominate the
// preference terms (which only break ties between equally clear candidates;
// their combined range is well below one covered pixel of peer line).
/// Per pixel of a peer edge's line hidden under the knockout. High: covering
/// a foreign line is exactly the defect this milestone eliminates.
const COVER_W: f32 = 25.0;
/// Per px² of another label's knockout overlapped, plus a flat term.
const LABEL_OVERLAP_W: f32 = 0.35;
const LABEL_OVERLAP_FLAT: f32 = 30.0;
/// Per px² of node interior covered, plus a flat term (heaviest: a label on a
/// node reads as part of the node).
const NODE_OVERLAP_W: f32 = 1.0;
const NODE_OVERLAP_FLAT: f32 = 120.0;
/// Per px² of frame title covered, plus a flat term. The frame *border* is
/// deliberately absent (M13.1): a label may cross a group's outline freely —
/// only the title text is kept clear.
const FRAME_OVERLAP_W: f32 = 0.4;
const FRAME_OVERLAP_FLAT: f32 = 40.0;
/// The rendered arrowhead's extent along its edge's final segment, back from
/// the tip (the renderer's `markerHeight`, 10 px with
/// `markerUnits="userSpaceOnUse"` — so px; the marker's base sits this far
/// short of the tip). The tip sits on the target node's boundary.
const ARROW_LEN: f32 = 10.0;
/// The rendered arrowhead's half-spread perpendicular to its segment at the
/// base (the renderer's `markerWidth` / 2 = 5 px): the triangle tapers to the
/// tip, and this half-spread bounds its width.
const ARROW_HALF_W: f32 = 5.0;
/// Padding around the arrowhead's bounding rect so a knockout never merely
/// grazes the marker's stroke.
const ARROW_PAD: f32 = 1.0;
/// Per px² of a rendered arrowhead covered, plus a flat term. Heavier than a
/// frame title and approaching a node: a knockout hiding an arrowhead reads
/// as a broken edge (no visible target arrow).
const ARROW_OVERLAP_W: f32 = 1.2;
const ARROW_OVERLAP_FLAT: f32 = 80.0;
/// Preference terms: distance from the segment midpoint (keep labels
/// centered on their run) and the graze variants' penalty (the on-line look
/// wins wherever nothing collides).
const TAP_W: f32 = 6.0;
const OFFSET_W: f32 = 8.0;

/// The static obstacles a label must dodge (M13): node interiors, group
/// frame title rects, and every rendered arrowhead's rect (see
/// [`arrowhead_rect`]). Deliberately absent: group frame border bands — a
/// label may cross a group's outline (M13.1). Peer edge segments and other
/// labels' knockouts are handled separately — the former per-edge (a label
/// is *supposed* to break its own line), the latter by the greedy and
/// refinement passes.
struct LabelWorld {
    nodes: Vec<(f32, f32, f32, f32)>,
    frames: Vec<(f32, f32, f32, f32)>,
    arrows: Vec<(f32, f32, f32, f32)>,
}

/// One label's winning candidate: the anchor, its preference cost, and its
/// total cost (collisions + preference).
struct LabelChoice {
    anchor: (f32, f32),
    pref: f32,
    total: f32,
}

/// A placed edge label: its edge index and its `(x, y, w, h)` knockout rect
/// in world space. Shared by the greedy pass, the refinement pass, and the
/// tests that re-run the same placement logic.
type PlacedLabel = (usize, (f32, f32, f32, f32));

/// Intersection area of two `(x, y, w, h)` rects (0 when disjoint).
fn rect_overlap_area(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> f32 {
    let w = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let h = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

/// The length of the segment `p0`→`p1` hidden inside rect `r` — what a
/// knockout placed there would cover of that line (Liang–Barsky via
/// [`segment_rect_interval`]).
fn segment_covered_len(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> f32 {
    let len = (p1.0 - p0.0).hypot(p1.1 - p0.1);
    match segment_rect_interval(p0, p1, r) {
        Some((t0, t1)) if t1 > t0 => (t1 - t0).min(1.0) * len,
        _ => 0.0,
    }
}

/// The rect covering the rendered arrowhead at the end of a routed polyline
/// (M13.1 — the label placement engine keeps knockouts off it): the marker
/// triangle whose tip sits on the last waypoint (the target node's
/// boundary), whose base sits [`ARROW_LEN`] back along the final segment and
/// spreads [`ARROW_HALF_W`] perpendicular — its bounding rect, padded by
/// [`ARROW_PAD`]. `None` for a polyline without a usable final segment.
fn arrowhead_rect(points: &[(f32, f32)]) -> Option<(f32, f32, f32, f32)> {
    let n = points.len();
    if n < 2 {
        return None;
    }
    let (tip, prev) = (points[n - 1], points[n - 2]);
    let len = (tip.0 - prev.0).hypot(tip.1 - prev.1);
    if len < 1e-3 {
        return None;
    }
    let (ux, uy) = ((tip.0 - prev.0) / len, (tip.1 - prev.1) / len);
    let base = (tip.0 - ARROW_LEN * ux, tip.1 - ARROW_LEN * uy);
    let (nx, ny) = (-uy, ux);
    let c1 = (base.0 + ARROW_HALF_W * nx, base.1 + ARROW_HALF_W * ny);
    let c2 = (base.0 - ARROW_HALF_W * nx, base.1 - ARROW_HALF_W * ny);
    let min_x = tip.0.min(c1.0).min(c2.0);
    let max_x = tip.0.max(c1.0).max(c2.0);
    let min_y = tip.1.min(c1.1).min(c2.1);
    let max_y = tip.1.max(c1.1).max(c2.1);
    Some(inflate(
        (min_x, min_y, max_x - min_x, max_y - min_y),
        ARROW_PAD,
    ))
}

/// Candidate anchors for a label of knockout `size` along `points` (M13) —
/// "offsets along segments": for each segment, the taps in [`LABEL_TAPS`]
/// (clamped so the knockout fits inside the segment where it is long enough
/// to hold it), each in up to three variants — centered on the line (the
/// classic knockout-breaks-the-line look) and the two perpendicular grazes
/// (the knockout's edge still crosses the line by [`LABEL_ON_LINE`], so the
/// break reads, but its bulk clears one side — the escape hatch for a crowded
/// parallel run). Returned in preference order — longest segment first (its
/// midpoint is the classic choice), then by tap, on-line before the grazes —
/// each with its preference cost; equal-cost ties in [`best_label_anchor`]
/// go to the earlier candidate.
fn label_candidates(points: &[(f32, f32)], size: (f32, f32)) -> Vec<((f32, f32), f32)> {
    let mut segs: Vec<(usize, f32)> = points
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)))
        .filter(|&(_, l)| l > 1e-3)
        .collect();
    segs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<((f32, f32), f32)> = Vec::new();
    for &(si, len) in &segs {
        let (a, b) = (points[si], points[si + 1]);
        let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        // Axis-aligned (the routed case): `along` is the knockout's extent
        // along the run (rw for a horizontal one, rh for a vertical one) and
        // `perp` its extent across it; the support form also covers a stray
        // diagonal segment.
        let along = (ux.abs() * size.0 + uy.abs() * size.1).max(1e-3);
        let perp = (uy.abs() * size.0 + ux.abs() * size.1).max(1e-3);
        for &t in &LABEL_TAPS {
            let mut s = t * len;
            if len >= along {
                s = s.clamp(along / 2.0, len - along / 2.0);
            }
            let p = (a.0 + ux * s, a.1 + uy * s);
            let tap_cost = TAP_W * (2.0 * (s / len) - 1.0).abs();
            push_unique(&mut out, (p, tap_cost));
            let d = perp / 2.0 - LABEL_ON_LINE;
            if d > 0.5 {
                let n = (-uy, ux);
                push_unique(
                    &mut out,
                    ((p.0 + n.0 * d, p.1 + n.1 * d), tap_cost + OFFSET_W),
                );
                push_unique(
                    &mut out,
                    ((p.0 - n.0 * d, p.1 - n.1 * d), tap_cost + OFFSET_W),
                );
            }
        }
    }
    out
}

/// Append candidate `c` unless an essentially identical anchor is already
/// present (clamped taps on a short segment collapse onto each other).
fn push_unique(out: &mut Vec<((f32, f32), f32)>, c: ((f32, f32), f32)) {
    if out
        .iter()
        .any(|(p, _)| (p.0 - c.0.0).abs() < 1e-3 && (p.1 - c.0.1).abs() < 1e-3)
    {
        return;
    }
    out.push(c);
}

/// The collision cost of putting a label's knockout rect `r` at a candidate
/// (M13): peer edge lines hidden under it (every edge but the label's own —
/// the knockout is *supposed* to break its own line), overlap with other
/// labels' knockouts, node interiors, and frame borders/titles. A pure
/// function of the rect — the preference terms ride on the candidate — so the
/// greedy and refinement passes compare like with like.
fn label_cost(
    r: (f32, f32, f32, f32),
    own: usize,
    paths: &[Vec<(f32, f32)>],
    world: &LabelWorld,
    placed: &[PlacedLabel],
) -> f32 {
    let mut c = 0.0_f32;
    let hit = inflate(r, LABEL_HIT_PAD);
    for (pi, pts) in paths.iter().enumerate() {
        if pi == own {
            continue;
        }
        for w in pts.windows(2) {
            c += COVER_W * segment_covered_len(w[0], w[1], hit);
        }
    }
    for &(_, lr) in placed {
        let a = rect_overlap_area(r, lr);
        if a > 1e-6 {
            c += LABEL_OVERLAP_FLAT + LABEL_OVERLAP_W * a;
        }
    }
    for &n in &world.nodes {
        let a = rect_overlap_area(r, n);
        if a > 1e-6 {
            c += NODE_OVERLAP_FLAT + NODE_OVERLAP_W * a;
        }
    }
    for &f in &world.frames {
        let a = rect_overlap_area(r, f);
        if a > 1e-6 {
            c += FRAME_OVERLAP_FLAT + FRAME_OVERLAP_W * a;
        }
    }
    for &a in &world.arrows {
        let a = rect_overlap_area(r, a);
        if a > 1e-6 {
            c += ARROW_OVERLAP_FLAT + ARROW_OVERLAP_W * a;
        }
    }
    c
}

/// The minimum-total-cost candidate for one label. Ties go to the earlier
/// candidate — the generation order of [`label_candidates`] *is* the
/// preference order — so the result is deterministic.
fn best_label_anchor(
    points: &[(f32, f32)],
    size: (f32, f32),
    own: usize,
    paths: &[Vec<(f32, f32)>],
    world: &LabelWorld,
    placed: &[PlacedLabel],
) -> Option<LabelChoice> {
    let mut best: Option<LabelChoice> = None;
    for (anchor, pref) in label_candidates(points, size) {
        let total = label_cost(label_rect(anchor, size), own, paths, world, placed) + pref;
        let better = match &best {
            Some(b) => total < b.total - 1e-6,
            None => true,
        };
        if better {
            best = Some(LabelChoice {
                anchor,
                pref,
                total,
            });
        }
    }
    best
}

/// The pre-M13 anchor — the midpoint of the polyline's longest segment — as
/// the fallback for a degenerate polyline without any candidate segment.
fn fallback_label_anchor(points: &[(f32, f32)]) -> (f32, f32) {
    if points.is_empty() {
        return (0.0, 0.0);
    }
    if points.len() == 1 {
        return points[0];
    }
    let mut best_i = 0usize;
    let mut best_len = -1.0_f32;
    for (i, w) in points.windows(2).enumerate() {
        let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if l > best_len {
            best_len = l;
            best_i = i;
        }
    }
    let (a, b) = (points[best_i], points[best_i + 1]);
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// Place every labeled edge's label (M13). `labels` and `paths` are in
/// declaration order (one entry per edge; `None` for an unlabeled edge),
/// `world` carries the static obstacles. Returns each edge's anchor (`None`
/// for unlabeled edges), ready to store on the [`EdgePath`].
///
/// Two deterministic passes, both in declaration order: a greedy pass where
/// each label takes the minimum-cost candidate given the labels placed before
/// it, then one refinement pass where each label re-optimizes against every
/// *other* label's current rect and moves only on a strict improvement (so a
/// later label that crowded an earlier one can be relieved by moving the
/// earlier one to its next-best spot).
fn place_labels(
    labels: &[Option<&str>],
    paths: &[Vec<(f32, f32)>],
    world: &LabelWorld,
) -> Vec<Option<(f32, f32)>> {
    let mut anchors: Vec<Option<(f32, f32)>> = (0..labels.len()).map(|_| None).collect();
    let mut prefs: Vec<f32> = (0..labels.len()).map(|_| 0.0).collect();
    let mut placed: Vec<PlacedLabel> = Vec::new();
    for ei in 0..labels.len() {
        let Some(label) = labels[ei] else { continue };
        let size = label_box(label);
        let choice =
            best_label_anchor(&paths[ei], size, ei, paths, world, &placed).unwrap_or(LabelChoice {
                anchor: fallback_label_anchor(&paths[ei]),
                pref: 0.0,
                total: 0.0,
            });
        anchors[ei] = Some(choice.anchor);
        prefs[ei] = choice.pref;
        placed.push((ei, label_rect(choice.anchor, size)));
    }
    for ei in 0..labels.len() {
        let Some(label) = labels[ei] else { continue };
        let size = label_box(label);
        let Some(cur) = anchors[ei] else { continue };
        let others: Vec<PlacedLabel> = placed.iter().filter(|(j, _)| *j != ei).cloned().collect();
        let cur_total = label_cost(label_rect(cur, size), ei, paths, world, &others) + prefs[ei];
        if let Some(ch) = best_label_anchor(&paths[ei], size, ei, paths, world, &others)
            && ch.total < cur_total - 1e-3
        {
            anchors[ei] = Some(ch.anchor);
            prefs[ei] = ch.pref;
            for p in placed.iter_mut() {
                if p.0 == ei {
                    p.1 = label_rect(ch.anchor, size);
                }
            }
        }
    }
    anchors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Shape;
    use crate::parser;
    use crate::resolve;

    // ---- M20.1: solid/ancestor classifier + track-region widening ------

    /// Parse + resolve a source, returning the resolved diagram (for
    /// classifier unit tests).
    fn diagram_of(src: &str) -> crate::ast::Diagram {
        let raw = parser::parse_diagram(src).expect("parse");
        resolve::resolve(&raw).expect("resolve")
    }

    /// Node indices for an id list, and the two endpoints' group
    /// memberships.
    fn memberships(d: &crate::ast::Diagram, ids: &[&str]) -> Vec<Option<usize>> {
        ids.iter()
            .map(|id| d.nodes.iter().find(|n| n.id == *id).expect("node").group)
            .collect()
    }

    /// The full pipeline (`phase1` stops after the M16 pass's simultaneous
    /// phase — phase 2's negotiation is declaration-order-dependent by
    /// design).
    fn lay(src: &str, phase1: bool) -> (crate::ast::Diagram, Layout) {
        let raw = parser::parse_diagram(src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let l = if phase1 {
            layout_phase1(&d)
        } else {
            layout(&d)
        };
        (d, l)
    }

    /// Convenience: the full pipeline (phase 2 included).
    fn lay_final(src: &str) -> (crate::ast::Diagram, Layout) {
        lay(src, false)
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

    fn sub_rect(l: &Layout, idx: usize) -> &GroupRect {
        l.groups
            .iter()
            .find(|s| s.index == idx)
            .unwrap_or_else(|| panic!("no group frame #{idx}"))
    }

    /// Do two axis-aligned rectangles overlap (with a tiny epsilon)?
    fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        ax < bx + bw - 1e-3 && bx < ax + aw - 1e-3 && ay < by + bh - 1e-3 && by < ay + ah - 1e-3
    }

    /// `b` fully contains `a` (with a tiny epsilon).
    fn contains(outer: (f32, f32, f32, f32), inner: (f32, f32, f32, f32)) -> bool {
        let (ox, oy, ow, oh) = outer;
        let (ix, iy, iw, ih) = inner;
        ix >= ox - 1e-3 && iy >= oy - 1e-3 && ix + iw <= ox + ow + 1e-3 && iy + ih <= oy + oh + 1e-3
    }

    /// Assert no two *real* node rectangles overlap.
    fn assert_no_overlaps(l: &Layout) {
        for i in 0..l.nodes.len() {
            for j in (i + 1)..l.nodes.len() {
                let a = &l.nodes[i];
                let b = &l.nodes[j];
                assert!(
                    !overlaps((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h),),
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
            if let Some((ax, ay)) = e.label_at {
                assert!(
                    ax.is_finite() && ay.is_finite(),
                    "{}->{} label anchor",
                    e.from,
                    e.to
                );
            }
        }
        for s in &l.groups {
            assert!(s.w > 0.0 && s.h > 0.0, "group {} has zero size", s.index);
        }
        assert!(l.width.is_finite() && l.height.is_finite());
        assert!(l.width > 0.0 && l.height > 0.0);
    }

    #[test]
    fn infra_example_layouts() {
        let (_d, l) = lay_final(include_str!("../examples/infra.dgmr"));
        assert_eq!(l.nodes.len(), 8);
        assert_eq!(l.edges.len(), 10);
        assert_eq!(l.groups.len(), 1);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_subdirection_for_inspection() {
        let (d, l) = lay_final(include_str!("../examples/subdirection.dgmr"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap()
                .then(a.x.partial_cmp(&b.x).unwrap())
        });
        for n in &nodes {
            println!(
                "  {:<14} x={:7.1} y={:7.1} w={:5.1} h={:5.1}",
                n.id, n.x, n.y, n.w, n.h
            );
        }
        for s in &l.groups {
            println!(
                "  group #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}",
                s.index, d.groups[s.index].title, s.x, s.y, s.w, s.h
            );
        }
        for e in &l.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_infra_for_inspection() {
        let (d, l) = lay_final(&if let Ok(p) = std::env::var("DIAG_SRC") {
            std::fs::read_to_string(p).unwrap()
        } else {
            include_str!("../examples/infra.dgmr").to_string()
        });
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap()
                .then(a.x.partial_cmp(&b.x).unwrap())
        });
        for n in &nodes {
            println!(
                "  {:<14} x={:7.1} y={:7.1} w={:5.1} h={:5.1}",
                n.id, n.x, n.y, n.w, n.h
            );
        }
        for s in &l.groups {
            println!(
                "  group #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}",
                s.index, d.groups[s.index].title, s.x, s.y, s.w, s.h
            );
        }
        for e in &l.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn every_node_has_a_rect() {
        let (d, l) = lay_final(include_str!("../examples/infra.dgmr"));
        for n in &d.nodes {
            assert!(
                l.nodes.iter().any(|r| r.id == n.id),
                "missing rect for {}",
                n.id
            );
        }
    }

    #[test]
    fn empty_diagram_is_empty_layout() {
        let (_d, l) = lay_final("diagram top-down\n");
        assert!(l.nodes.is_empty());
        assert!(l.edges.is_empty());
        assert_eq!(l.width, 0.0);
        assert_eq!(l.height, 0.0);
    }

    #[test]
    fn isolated_nodes_follow_the_flow_axis() {
        // Three disconnected nodes, top-down: component packing re-bases each
        // rank-0 singleton onto its own band, so they stack along the flow
        // axis (top-to-bottom) in declaration order — the declared direction
        // is honored even with no edges at all.
        let (_d, l) = lay_final("diagram top-down\na\nb\nc\n");
        assert_eq!(l.nodes.len(), 3);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // One column ⇒ same left x (all singletons have equal width → same center → same left).
        let x0 = cx(&l.nodes[0]);
        assert!(l.nodes.iter().all(|n| (cx(n) - x0).abs() < 1e-3));
        let ys: Vec<f32> = l.nodes.iter().map(|n| n.y).collect();
        assert!(
            ys[0] < ys[1] && ys[1] < ys[2],
            "expected top-to-bottom order {ys:?}"
        );
    }

    #[test]
    fn isolated_nodes_run_horizontally_left_right() {
        // The same three edge-less nodes under left-right: they run
        // left-to-right, mirroring the top-down stacking.
        let (_d, l) = lay_final("diagram left-right\na\nb\nc\n");
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        let y0 = l.nodes[0].y;
        assert!(l.nodes.iter().all(|n| (n.y - y0).abs() < 1e-3));
        let xs: Vec<f32> = l.nodes.iter().map(|n| n.x).collect();
        assert!(
            xs[0] < xs[1] && xs[1] < xs[2],
            "expected left-to-right order {xs:?}"
        );
    }

    #[test]
    fn chain_aligns_vertically_top_down() {
        let (_d, l) = lay_final("diagram top-down\na-->b-->c-->d\n");
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
        let (_d, l) = lay_final("diagram left-right\na-->b-->c\n");
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
        let (_d, l) = lay_final("diagram left-right\nauth --> users --> orders\n");
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
        let (_d, l) = lay_final("diagram left-right\nalpha --> beta\n");
        let a = node_rect(&l, "alpha");
        let b = node_rect(&l, "beta");
        assert!(a.x < b.x, "alpha should be left of beta");
        let e = edge_path(&l, "alpha", "beta");
        let (p0, p1) = (e.points[0], *e.points.last().unwrap());
        // Start on alpha's right edge, at its vertical center.
        assert!(
            (p0.0 - (a.x + a.w)).abs() < 1e-2,
            "start not on alpha's right: {p0:?} a={a:?}"
        );
        assert!((p0.1 - (a.y + a.h / 2.0)).abs() < 1e-2);
        // End on beta's left edge, at its vertical center.
        assert!(
            (p1.0 - b.x).abs() < 1e-2,
            "end not on beta's left: {p1:?} b={b:?}"
        );
        assert!((p1.1 - (b.y + b.h / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn direction_swaps_dimensions_for_a_chain() {
        let (_d, l_td) = lay_final("diagram top-down\na-->b-->c-->d\n");
        let (_d, l_lr) = lay_final("diagram left-right\na-->b-->c-->d\n");
        // A 4-rank chain is tall top-down, wide left-right.
        assert!(l_td.height > l_td.width, "top-down chain should be tall");
        assert!(l_lr.width > l_lr.height, "left-right chain should be wide");
    }

    #[test]
    fn cycle_is_handled_without_overlap() {
        // A <-> B (a 2-cycle). One edge is reversed internally; both still draw.
        let (_d, l) = lay_final("diagram top-down\na-->b\nb-->a\n");
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
        let (_d, l) = lay_final("diagram top-down\na-->b\nc\nb-->c\na-->c\n");
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
        let (_d, l) = lay_final("diagram top-down\na-->a\n");
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
        let (_d, l) = lay_final(src);
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
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
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
        let (_d, l_td) = lay_final(src);
        let (_d, l_bu) = lay_final("diagram bottom-up\na-->b-->c\n");
        // In top-down, a is at the top (smallest y); in bottom-up, a is at the
        // bottom (largest y).
        let a_td = node_rect(&l_td, "a");
        let a_bu = node_rect(&l_bu, "a");
        assert!(
            a_td.y < a_bu.y,
            "a should be higher in top-down than bottom-up"
        );
        let c_td = node_rect(&l_td, "c");
        let c_bu = node_rect(&l_bu, "c");
        assert!(
            c_bu.y < c_td.y,
            "c should be lower in bottom-up than top-down"
        );
    }

    #[test]
    fn right_left_reverses_left_right() {
        let (_d, l_lr) = lay_final("diagram left-right\na-->b-->c\n");
        let (_d, l_rl) = lay_final("diagram right-left\na-->b-->c\n");
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
        let (_d, l) = lay_final("diagram top-down\na-->b\n");
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let e = edge_path(&l, "a", "b");
        let (p0, p1) = (e.points[0], e.points[1]);
        // First point on a's bottom edge, at its horizontal center.
        assert!(
            (p0.1 - (a.y + a.h)).abs() < 1e-2,
            "start not on a's bottom: {p0:?} a={a:?}"
        );
        assert!((p0.0 - (a.x + a.w / 2.0)).abs() < 1e-2);
        // Last point on b's top edge, at its horizontal center.
        assert!(
            (p1.1 - b.y).abs() < 1e-2,
            "end not on b's top: {p1:?} b={b:?}"
        );
        assert!((p1.0 - (b.x + b.w / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn cylinder_is_taller_than_a_box_for_the_same_label() {
        // A cylinder needs room for its elliptical caps plus breathing space
        // for the centered label (see `cyl_height`), so it is taller than a
        // box with the same label — and tall enough that the label clears the
        // lid (the guarantee exercised in the render tests).
        let (_d, l) = lay_final(
            "diagram top-down\n\n\
             box \"Pg\"\n\
             cyl \"Pg\" : cylinder\n",
        );
        let box_h = node_rect(&l, "box").h;
        let cyl_h = node_rect(&l, "cyl").h;
        assert!(
            cyl_h > box_h,
            "cylinder ({cyl_h}) should be taller than box ({box_h})"
        );
        // Concretely: box = 3·FONT_SIZE = 42; cylinder adds 2·CYL_PAD + 2·CYL_RY.
        assert_eq!(box_h, 42.0);
        assert_eq!(cyl_h, 42.0 + 2.0 * CYL_PAD + 2.0 * CYL_RY);
        // The label clears the lid's lowest point (em box above it).
        let r = node_rect(&l, "cyl");
        let cy = r.y + r.h / 2.0;
        let glyph_top = cy - FONT_SIZE / 2.0;
        let lid_bottom = r.y + 2.0 * CYL_RY;
        assert!(
            glyph_top >= lid_bottom + 3.0,
            "label would collide with lid"
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        let _ = Shape::Cylinder;
    }

    // ================= Compound layout (M4) =================

    #[test]
    fn group_frame_contains_its_members() {
        let (d, l) = lay_final(
            "diagram top-down\n\
             group \"Cluster\"\n\
             a\n\
             b\n\
             end\n",
        );
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.nodes.len(), 2);
        let f = sub_rect(&l, 0);
        assert_eq!(d.groups[0].title.as_deref(), Some("Cluster"));
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
        // A long title on a narrow group: the frame must widen so the
        // title text fits, rather than the title spilling out of the frame.
        let (d, l) = lay_final(
            "diagram top-down\n\
             group \"A Very Long Cluster Title\"\n\
             a\n\
             end\n",
        );
        assert_eq!(l.groups.len(), 1);
        let f = sub_rect(&l, 0);
        let title = d.groups[0].title.as_deref().unwrap();
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
        let (_d, l) = lay_final("diagram top-down\napp \"Kubernetes\\nCluster\"\n");
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
        let (_d1, l1) = lay_final(
            "diagram top-down\n\
             group \"Cluster\"\n\
             a\n\
             end\n",
        );
        let (_d2, l2) = lay_final(
            "diagram top-down\n\
             group \"Kubernetes\\nCluster\"\n\
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
    fn nested_group_frame_inside_outer_frame() {
        let (d, l) = lay_final(
            "diagram top-down\n\
             group \"Outer\"\n\
             group \"Inner\"\n\
             a\n\
             end\n\
             end\n",
        );
        assert_eq!(l.groups.len(), 2);
        let outer = sub_rect(&l, 0);
        let inner = sub_rect(&l, 1);
        assert_eq!(d.groups[0].title.as_deref(), Some("Outer"));
        assert_eq!(d.groups[1].title.as_deref(), Some("Inner"));
        assert!(
            contains(
                (outer.x, outer.y, outer.w, outer.h),
                (inner.x, inner.y, inner.w, inner.h)
            ),
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
    fn group_inherits_diagram_direction() {
        // A group with no explicit direction inherits the diagram's
        // direction. With left-right, two edge-less members now follow that
        // inherited flow axis: they run left-to-right (side by side), not
        // stacked.
        let (_d, l) = lay_final(
            "diagram left-right\n\
             group \"S\"\n\
             a\n\
             b\n\
             end\n",
        );
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        // left-right ⇒ ranks flow along x, so the two isolated members are
        // side by side ⇒ distinct x, same y-center.
        assert!(
            a.x < b.x,
            "members should run horizontally under left-right"
        );
        assert!(((a.y + a.h / 2.0) - (b.y + b.h / 2.0)).abs() < 1e-2);
    }

    #[test]
    fn per_group_direction_changes_internal_arrangement() {
        // Same internal graph (a chain) under two different group
        // directions. Top-down ⇒ frame is tall; left-right ⇒ frame is wide.
        let src_td = "diagram top-down\n\
             group top-down \"S\"\n\
             a --> b --> c\n\
             end\n";
        let src_lr = "diagram top-down\n\
             group left-right \"S\"\n\
             a --> b --> c\n\
             end\n";
        let (_d, l_td) = lay_final(src_td);
        let (_d, l_lr) = lay_final(src_lr);
        let f_td = sub_rect(&l_td, 0);
        let f_lr = sub_rect(&l_lr, 0);

        // Internal chain geometry: top-down ⇒ a above b above c (distinct y);
        // left-right ⇒ a left of b left of c (distinct x).
        let (a_td, b_td, c_td) = (
            node_rect(&l_td, "a"),
            node_rect(&l_td, "b"),
            node_rect(&l_td, "c"),
        );
        assert!(
            a_td.y < b_td.y && b_td.y < c_td.y,
            "top-down group should stack vertically"
        );
        let (a_lr, b_lr, c_lr) = (
            node_rect(&l_lr, "a"),
            node_rect(&l_lr, "b"),
            node_rect(&l_lr, "c"),
        );
        assert!(
            a_lr.x < b_lr.x && b_lr.x < c_lr.x,
            "left-right group should run horizontally"
        );

        // The frame aspect ratio flips between the two.
        assert!(
            f_td.h > f_td.w,
            "top-down group frame should be tall (w={:.1} h={:.1})",
            f_td.w,
            f_td.h
        );
        assert!(
            f_lr.w > f_lr.h,
            "left-right group frame should be wide (w={:.1} h={:.1})",
            f_lr.w,
            f_lr.h
        );
    }

    #[test]
    fn top_level_nodes_and_group_coexist_without_overlap() {
        let (_d, l) = lay_final(
            "diagram top-down\n\
             x\n\
             group \"S\"\n\
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
            "top-level node x overlaps group frame"
        );
    }

    #[test]
    fn cross_boundary_edge_renders_between_actual_endpoints() {
        // lb (top-level) -> api (inside group): a cross-boundary edge.
        let (_d, l) = lay_final(
            "diagram top-down\n\
             lb\n\
             group \"S\"\n\
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
        assert!(
            on_lb,
            "cross-boundary edge start not on lb's boundary: {p0:?}"
        );
        let on_api = (p1.0 - api.x).abs() < 1e-2
            || (p1.0 - (api.x + api.w)).abs() < 1e-2
            || (p1.1 - api.y).abs() < 1e-2
            || (p1.1 - (api.y + api.h)).abs() < 1e-2;
        assert!(
            on_api,
            "cross-boundary edge end not on api's boundary: {p1:?}"
        );
    }

    #[test]
    fn internal_group_edge_uses_internal_layout() {
        // An edge fully inside a group is a direct internal edge: laid out
        // within the group, so b sits strictly below a (top-down group).
        let (_d, l) = lay_final(
            "diagram top-down\n\
             group \"S\"\n\
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
        // The headline guarantee: a group with its own direction keeps
        // that internal arrangement even when edges cross its boundary.
        // Here the group is left-right with a chain; external edges attach
        // to its members but the chain still runs horizontally.
        let (_d, l) = lay_final(
            "diagram top-down\n\
             src\n\
             sink\n\
             group left-right \"S\"\n\
             a --> b --> c\n\
             end\n\
             src --> a\n\
             c --> sink\n",
        );
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        assert!(
            a.x < b.x && b.x < c.x,
            "left-right group chain must stay horizontal"
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // Both cross-boundary edges render.
        assert!(edge_path(&l, "src", "a").points.len() >= 2);
        assert!(edge_path(&l, "c", "sink").points.len() >= 2);
    }

    #[test]
    fn every_descendant_node_present_with_nested_groups() {
        let (_d, l) = lay_final(
            "diagram top-down\n\
             group \"A\"\n\
             group \"B\"\n\
             group \"C\"\n\
             x --> y\n\
             end\n\
             end\n\
             end\n",
        );
        assert_eq!(l.nodes.len(), 2);
        assert_eq!(l.groups.len(), 3);
        assert_all_finite(&l);
        // x -> y is a direct internal edge of the innermost group.
        assert!(edge_path(&l, "x", "y").points.len() >= 2);
        // Frames nest: A ⊃ B ⊃ C.
        let a = sub_rect(&l, 0);
        let b = sub_rect(&l, 1);
        let c = sub_rect(&l, 2);
        assert!(contains((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h)));
        assert!(contains((b.x, b.y, b.w, b.h), (c.x, c.y, c.w, c.h)));
    }

    #[test]
    fn empty_group_renders_a_frame() {
        let (d, l) = lay_final(
            "diagram top-down\n\
             group \"Empty\"\n\
             end\n",
        );
        assert_eq!(l.groups.len(), 1);
        let f = sub_rect(&l, 0);
        assert_eq!(d.groups[0].title.as_deref(), Some("Empty"));
        assert!(
            f.w > 0.0 && f.h > 0.0,
            "empty group should still have a visible frame"
        );
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
    fn segment_intersects_rect(p0: (f32, f32), p1: (f32, f32), r: (f32, f32, f32, f32)) -> bool {
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

    /// A node/group rect as a tuple, for the geometry helpers above.
    fn rect_of(n: &NodeRect) -> (f32, f32, f32, f32) {
        (n.x, n.y, n.w, n.h)
    }
    fn sub_rect_of(s: &GroupRect) -> (f32, f32, f32, f32) {
        (s.x, s.y, s.w, s.h)
    }
    #[test]
    fn cross_boundary_frame_points_preserve_group_direction() {
        // The headline M5 guarantee: a group's own direction is kept even
        // when edges cross its frame, AND those edges route through frame
        // connection points rather than piercing the frame.
        let (_d, l) = lay_final(
            r#"diagram top-down
src
sink
group left-right "S"
a --> b --> c
end
src --> a
c --> sink
"#,
        );
        // Per-group direction preserved: the chain stays horizontal.
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let c = node_rect(&l, "c");
        assert!(
            a.x < b.x && b.x < c.x,
            "left-right group chain must stay horizontal"
        );
        // Both cross-boundary edges route through S frame connection points.
        let s = sub_rect(&l, 0);
        let s_rect = sub_rect_of(s);
        assert!(
            edge_path(&l, "src", "a")
                .points
                .iter()
                .any(|&p| on_boundary(s_rect, p)),
            "src->a should have a connection point on S's frame"
        );
        assert!(
            edge_path(&l, "c", "sink")
                .points
                .iter()
                .any(|&p| on_boundary(s_rect, p)),
            "c->sink should have a connection point on S's frame"
        );
        assert_all_finite(&l);
        assert_no_overlaps(&l);
    }
    #[test]
    fn cross_boundary_edge_crosses_each_nested_frame() {
        // ext (top-level) -> inner (inside Inner inside Outer): a cross-
        // boundary edge crossing two nested frames. It must record a
        // connection point on each frame it passes through.
        let (_d, l) = lay_final(
            r#"diagram top-down
ext
group "Outer"
group "Inner"
inner
end
end
ext --> inner
"#,
        );
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

    /// Do two axis-aligned segments conflict the way sibling routes must
    /// not: a perpendicular crossing, or a parallel run closer than
    /// [`SEGMENT_OBSTACLE_PAD`] over a shared span (collinear riding).
    fn segments_conflict(a0: (f32, f32), a1: (f32, f32), b0: (f32, f32), b1: (f32, f32)) -> bool {
        let overlaps = |a0: f32, a1: f32, b0: f32, b1: f32| {
            let (alo, ahi) = (a0.min(a1), a0.max(a1));
            let (blo, bhi) = (b0.min(b1), b0.max(b1));
            alo < bhi - 1e-6 && blo < ahi - 1e-6
        };
        let a_vert = (a0.0 - a1.0).abs() < 1e-6;
        let b_vert = (b0.0 - b1.0).abs() < 1e-6;
        if a_vert == b_vert {
            // Parallel: riding when the lines are closer than the pad over
            // an overlapping run.
            let (ca, cb) = if a_vert { (a0.0, b0.0) } else { (a0.1, b0.1) };
            let (ao, bo) = if a_vert {
                ((a0.1, a1.1), (b0.1, b1.1))
            } else {
                ((a0.0, a1.0), (b0.0, b1.0))
            };
            (ca - cb).abs() < SEGMENT_OBSTACLE_PAD - 1e-6 && overlaps(ao.0, ao.1, bo.0, bo.1)
        } else {
            // Perpendicular: a crossing when each run spans the other.
            let (h, v) = if a_vert {
                ((b0, b1), (a0, a1))
            } else {
                ((a0, a1), (b0, b1))
            };
            let cross = v.0.0;
            let row = h.0.1;
            overlaps(h.0.0, h.1.0, cross, cross) && overlaps(v.0.1, v.1.1, row, row)
        }
    }
    #[test]
    fn sibling_fan_edges_never_cross_or_ride() {
        // Three edges fanning from one node's side into a nested stack of
        // frames (the cluster.dgmr shape): the target n1 sits nearly level
        // with the source, so its entry row falls between the source fan's
        // rows. The lanes' turn order must then be the non-crossing one —
        // otherwise lb->n1 crosses lb->n2's source run and then lies almost
        // on top of it. No two of the three routes may cross, and no two
        // may run parallel within the riding pad.
        let (_d, l) = lay_final(
            r#"diagram top-down
user
group left-right
group "Application"
lb
end
group "Shared Services"
group top-down "Kubernetes Node Group"
n1
n2
n3
end
end
end
user --> lb
lb --> n1
lb --> n2
lb --> n3
n1 -- dotted --> n2 -- dotted --> n3
n3 -- dotted --> n2 -- dotted --> n1
"#,
        );
        let es: Vec<&EdgePath> = ["n1", "n2", "n3"]
            .iter()
            .map(|t| edge_path(&l, "lb", t))
            .collect();
        for i in 0..es.len() {
            for j in (i + 1)..es.len() {
                for a in es[i].points.windows(2) {
                    for b in es[j].points.windows(2) {
                        assert!(
                            !segments_conflict(a[0], a[1], b[0], b[1]),
                            "lb->{} segment {:?}->{:?} conflicts with lb->{} segment {:?}->{:?}",
                            ["n1", "n2", "n3"][i],
                            a[0],
                            a[1],
                            ["n1", "n2", "n3"][j],
                            b[0],
                            b[1]
                        );
                    }
                }
            }
        }
        assert_all_finite(&l);
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
        let long = title_clear_gap(
            text::measure("A Very Long Group Title Indeed", FRAME_TITLE_FONT_SIZE).width,
        );
        assert!(short > TITLE_CLEAR_GAP, "fixed bump missing: {short}");
        assert!(
            long > short,
            "length-derived component missing: {long} vs {short}"
        );
        assert!(
            long - short > 0.05 * (long_width_hint()),
            "per-length component too small: {long} vs {short}"
        );
    }

    #[test]
    fn frame_width_carries_the_fallback_margin_past_the_title() {
        // A titled frame is at least FRAME_TITLE_X + title_w +
        // title_clear_gap(title_w) wide: the measured title must fit with
        // the fallback-safe right margin — a viewer whose font stack
        // resolves wider than the measured face must not spill the title out
        // of the frame (the "Kubernetes Node Group" shape).
        let (_d, l) = lay_final(
            r#"diagram top-down
group "Kubernetes Node Group"
n1
n2
n3
end"#,
        );
        let s = sub_rect(&l, 0);
        let tw = text::measure("Kubernetes Node Group", FRAME_TITLE_FONT_SIZE).width;
        let need = FRAME_TITLE_X + tw + title_clear_gap(tw);
        assert!(
            s.w >= need - 1e-2,
            "frame {} too narrow for its title (need {})",
            s.w,
            need
        );
        // Sanity: the margin grows the frame past the old title-only fit.
        assert!(s.w > tw + 2.0 * FRAME_TITLE_X, "fallback margin missing");
    }

    /// Width of the long test title, for the proportional-gap assertion.
    fn long_width_hint() -> f32 {
        text::measure("A Very Long Group Title Indeed", FRAME_TITLE_FONT_SIZE).width
    }

    /// A wide-titled single-frame group with one small child `a`, plus a
    /// cross-boundary edge `src --> a`. The frame's width is determined by
    /// the title (the child is too narrow to widen it), and `a` sits under
    /// the title's left half — the shape of the M10 left-side detour.
    fn left_detour_setup() -> (crate::ast::Diagram, Layout) {
        lay_final(
            r#"diagram top-down
src
group "A Very Long Group Title Indeed"
a
end
src --> a
"#,
        )
    }

    /// The `x` at which the `src -> a` edge crosses the frame's top edge.
    fn frame_top_entry_x(l: &Layout, s: &GroupRect, to: &str) -> f32 {
        let e = edge_path(l, "src", to);
        e.points
            .iter()
            .find(|p| (p.1 - s.y).abs() < 1e-2)
            .unwrap_or_else(|| panic!("no connection point on the frame top: {:?}", e.points))
            .0
    }

    /// The title text rect of a titled frame (top band, starting at
    /// [`FRAME_TITLE_X`], the measured width wide, glyph height tall).
    fn title_rect(s: &GroupRect, title: &str) -> (f32, f32, f32, f32) {
        let tw = text::measure(title, FRAME_TITLE_FONT_SIZE).width;
        (s.x + FRAME_TITLE_X, s.y + 2.0, tw, FRAME_TITLE_H - 6.0)
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
        let (_d, l) = lay_final("diagram top-down\np\na\nbb\nccc\np-->a\np-->bb\np-->ccc\n");
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
        let (_d, l) = lay_final("diagram top-down\na\nb\nc\na-->c\nb-->c\n");
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
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
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
        let (_d, l) = lay_final(include_str!("../examples/infra.dgmr"));
        let web = node_rect(&l, "web");
        let lb = node_rect(&l, "lb");
        let k8s = sub_rect(&l, 0);
        let web_c = cx(web);
        assert!(
            (cx(lb) - web_c).abs() < 1e-2,
            "Load Balancer not aligned with Web Server"
        );
        let k8s_c = k8s.x + k8s.w / 2.0;
        assert!(
            (k8s_c - web_c).abs() < 1e-2,
            "K8s cluster frame not aligned with Web/LB"
        );
    }

    #[test]
    fn infra_data_tier_centers_under_cluster() {
        // Postgres + Redis + Message Queue center as a block under the K8s
        // cluster (and thus under Web/LB).
        let (_d, l) = lay_final(include_str!("../examples/infra.dgmr"));
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
        let (_d, l) = lay_final(include_str!("../examples/infra.dgmr"));
        let db = node_rect(&l, "db");
        let replica = node_rect(&l, "replica");
        assert!(
            (cx(replica) - cx(db)).abs() < 1e-2,
            "Replica not aligned under Postgres"
        );
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
            (include_str!("../examples/infra.dgmr"), "infra"),
            (
                include_str!("../examples/subdirection.dgmr"),
                "subdirection",
            ),
            (
                "diagram top-down\n\
                 src\n\
                 sink\n\
                 group left-right \"S\"\n\
                 a --> b --> c\n\
                 end\n\
                 src --> a\n\
                 c --> sink\n",
                "cross-boundary into left-right group",
            ),
        ];
        for (src, name) in cases {
            let (_d, l) = lay_final(src);
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
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        for (from, to) in [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")] {
            let e = edge_path(&l, from, to);
            assert!(
                e.points.len() > 2,
                "{from}->{to} should have bends: {:?}",
                e.points
            );
            assert!(
                is_orthogonal(&e.points),
                "{from}->{to} not orthogonal: {:?}",
                e.points
            );
        }
    }

    #[test]
    fn orthogonal_edge_endpoints_touch_node_bounds() {
        // Regression: orthogonalization must not move the first/last waypoint
        // off the source/target node boundary.
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
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
        let (_d, l) = lay_final(include_str!("../examples/infra.dgmr"));
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
    // ================= M9 — edge separation & obstacle avoidance =================

    /// Two axis-aligned segments share a collinear, overlapping span (i.e. the
    /// edges would lay on top of each other) — the failure mode M9 eliminates.
    fn segments_overlap(a0: (f32, f32), a1: (f32, f32), b0: (f32, f32), b1: (f32, f32)) -> bool {
        let same_x =
            (a0.0 - a1.0).abs() < 1e-2 && (b0.0 - b1.0).abs() < 1e-2 && (a0.0 - b0.0).abs() < 1e-2;
        let same_y =
            (a0.1 - a1.1).abs() < 1e-2 && (b0.1 - b1.1).abs() < 1e-2 && (a0.1 - b0.1).abs() < 1e-2;
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
                            !segments_overlap(
                                ei.points[k],
                                ei.points[k + 1],
                                ej.points[m],
                                ej.points[m + 1]
                            ),
                            "edges {}->{} and {}->{} overlap (seg {k} / {m})",
                            ei.from,
                            ei.to,
                            ej.from,
                            ej.to
                        );
                    }
                }
            }
        }
        // No edge rides over itself either (M11): two of its own segments
        // sharing a collinear overlapping span — an out-and-back spike, e.g.
        // a route doubling back over its own escape stub. Adjacent
        // perpendicular segments share only their corner, and the border
        // crossings re-inserted along a straight run ([
        // `record_border_crossings`]) are collinear but non-overlapping, so
        // neither false-positives.
        for e in &l.edges {
            for k in 0..e.points.len().saturating_sub(1) {
                for m in (k + 1)..e.points.len().saturating_sub(1) {
                    assert!(
                        !segments_overlap(
                            e.points[k],
                            e.points[k + 1],
                            e.points[m],
                            e.points[m + 1]
                        ),
                        "edge {}->{} overlaps itself (seg {k} / {m})",
                        e.from,
                        e.to
                    );
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
                        e.from,
                        e.to,
                        n.id
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_finance_for_inspection() {
        let (d, l) = lay_final(include_str!("../examples/finance.dgmr"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap()
                .then(a.x.partial_cmp(&b.x).unwrap())
        });
        for n in &nodes {
            println!(
                "  {:<12} x={:7.1} y={:7.1} w={:5.1} h={:5.1}",
                n.id, n.x, n.y, n.w, n.h
            );
        }
        for s in &l.groups {
            println!(
                "  group #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}",
                s.index, d.groups[s.index].title, s.x, s.y, s.w, s.h
            );
        }
        for e in &l.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  {:<10} -> {:<10} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn infra_edges_do_not_overlap_or_pass_through_nodes() {
        let (d, l) = lay_final(include_str!("../examples/infra.dgmr"));
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }

    #[test]
    fn subdirection_edges_do_not_overlap_or_pass_through_nodes() {
        let (d, l) = lay_final(include_str!("../examples/subdirection.dgmr"));
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }

    #[test]
    fn cluster_and_tencent_edges_do_not_overlap_or_pass_through_nodes() {
        // The two deepest cross-boundary samples (nested titled groups on
        // both sides of every edge) — the whole-path regime's hardest
        // geometry: the search must thread the same corridors the deleted
        // rep model hand-built, without riding a peer or puncturing a node.
        for src in [
            include_str!("../examples/cluster.dgmr"),
            include_str!("../examples/tencent.dgmr"),
        ] {
            let (d, l) = lay_final(src);
            assert_all_finite(&l);
            for e in &l.edges {
                assert!(
                    is_orthogonal(&e.points),
                    "{}->{} not orthogonal: {:?}",
                    e.from,
                    e.to,
                    e.points
                );
            }
            assert_no_edge_overlaps_or_node_passage(&d, &l);
        }
    }

    #[test]
    fn finance_edges_do_not_overlap_or_pass_through_nodes() {
        // The finance diagram: a User reaching a VPN and fanning out to many
        // cvms — the two M9 stress cases. The thick `User --> pubclb` edge
        // and the cross-boundary `User --> VPN` edge share User's bottom
        // side, and five `VPN --> cvm` edges share VPN's bottom.
        let (d, l) = lay_final(include_str!("../examples/finance.dgmr"));
        assert_all_finite(&l);
        assert_eq!(d.edges.len(), l.edges.len());
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
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
        // group S, below) with `mid` (a peer) sitting between them on the
        // same x as both endpoints, so a single midpoint jog cannot clear it.
        let (_d, l) = lay_final(
            r#"diagram top-down
src
mid
group "S"
t
end
src --> t
"#,
        );
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
                let (_d, l) = lay(&src, false);
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
                let (_d, l) = lay(&src, false);
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
        let (_d, l) = lay_final("diagram top-down\na -- from=\"top\" --> b\n");
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
        let (_d, l) = lay_final(
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
        assert!(
            d >= FAN_SEP - 1e-2,
            "forced-side fan separation {d:.2} < FAN_SEP"
        );
    }

    #[test]
    fn forced_side_excursion_grows_the_canvas() {
        // Forcing a side that points off the canvas edge (the page margin
        // is 20, the escape [`SIDE_ESCAPE`] is 24) routes outside the
        // laid-out content bounds; the canvas grows to contain every drawn
        // point — the renderer's viewBox comes straight from width/height.
        let (_d, l_nat) = lay_final("diagram top-down\na --> b\n");
        let (_d, l) = lay_final("diagram top-down\na -- from=\"left\" --> b\n");
        assert!(
            l.width > l_nat.width + 1e-2,
            "canvas did not grow for the forced excursion: {} vs {}",
            l.width,
            l_nat.width
        );
        // Everything drawn sits inside the (grown) canvas.
        for e in &l.edges {
            for &(px, py) in &e.points {
                assert!(
                    px >= -1e-3 && px <= l.width + 1e-3,
                    "point x {px} outside canvas"
                );
                assert!(
                    py >= -1e-3 && py <= l.height + 1e-3,
                    "point y {py} outside canvas"
                );
            }
        }
        for n in &l.nodes {
            assert!(n.x >= -1e-3 && n.x + n.w <= l.width + 1e-3);
            assert!(n.y >= -1e-3 && n.y + n.h <= l.height + 1e-3);
        }
    }

    #[test]
    fn canvas_keeps_the_page_margin_clear_of_every_object() {
        // Whatever routes where — forced-side escapes past the margin,
        // around-frame runs, label knockouts overhanging their line — every
        // drawn object (node rect, frame rect, edge waypoint, label
        // knockout) keeps at least [`MARGIN`] clear of every canvas edge.
        // (Strokes and arrowheads paint within a few px of the geometry
        // checked here; the margin absorbs that.)
        let cases: [&str; 4] = [
            include_str!("../examples/sides.dgmr"),
            include_str!("../examples/infra.dgmr"),
            include_str!("../examples/finance.dgmr"),
            "diagram top-down\na -- \"hit\" from=\"left\" --> b\n",
        ];
        for src in cases {
            let (d, l) = lay_final(src);
            let check = |what: &str, x0: f32, y0: f32, x1: f32, y1: f32| {
                assert!(
                    x0 >= MARGIN - 1e-2,
                    "{what}: left {x0:.2} inside the page margin"
                );
                assert!(
                    y0 >= MARGIN - 1e-2,
                    "{what}: top {y0:.2} inside the page margin"
                );
                assert!(
                    x1 <= l.width - MARGIN + 1e-2,
                    "{what}: right {x1:.2} inside the page margin (canvas {})",
                    l.width
                );
                assert!(
                    y1 <= l.height - MARGIN + 1e-2,
                    "{what}: bottom {y1:.2} inside the page margin (canvas {})",
                    l.height
                );
            };
            for n in &l.nodes {
                check(&n.id, n.x, n.y, n.x + n.w, n.y + n.h);
            }
            for s in &l.groups {
                check("frame", s.x, s.y, s.x + s.w, s.y + s.h);
            }
            for (i, e) in l.edges.iter().enumerate() {
                for &(px, py) in &e.points {
                    check("edge point", px, py, px, py);
                }
                if e.label_at.is_some() {
                    let (x, y, w, h) = label_knockout_of(&d, &l, i);
                    check("edge label", x, y, x + w, y + h);
                }
            }
        }
    }

    #[test]
    fn forced_self_loop_same_side_makes_a_bump_loop() {
        // Same-side loop: a rectangular bump hanging off the right side,
        // instead of the fixed below-node stub.
        let (_d, l) = lay_final("diagram top-down\na -- from=\"right\" to=\"right\" --> a\n");
        let a = node_rect(&l, "a");
        let e = edge_path(&l, "a", "a");
        assert!(on_side(a, e.points[0], "right"));
        assert!(on_side(a, *e.points.last().unwrap(), "right"));
        assert!(is_orthogonal(&e.points));
        for &(px, _) in &e.points[1..e.points.len() - 1] {
            assert!(
                px >= a.x + a.w - 1e-2,
                "loop interior point at {px} crosses a's body"
            );
        }
    }

    #[test]
    fn forced_self_loop_opposite_sides_wraps_a_corner() {
        // Opposite-side loop: out of the bottom, around one side, back into
        // the top.
        let (_d, l) = lay_final("diagram top-down\na -- from=\"bottom\" to=\"top\" --> a\n");
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
        // `src --> t` where t sits inside a group, forced to enter t at
        // its RIGHT side (M14): the port lands on t's right side while the
        // frame keeps its natural crossing (the top border, facing the
        // source) — the stub descends beside t and hooks into its right
        // side, and the forced side never drags the frame crossing around
        // the group.
        let (_d, l) = lay_final(
            r#"diagram top-down
src
group "S"
t "Target"
end
src -- to="right" --> t
"#,
        );
        let t = node_rect(&l, "t");
        let e = edge_path(&l, "src", "t");
        assert!(on_side(t, *e.points.last().unwrap(), "right"));
        assert!(is_orthogonal(&e.points));
        // The frame is entered through its top border, never wrapped around
        // to its right border.
        let s = &l.groups[0];
        for &(x, _) in &e.points {
            assert!(
                x < s.x + s.w - 1.0,
                "point x {x} wrapped around the frame's right border ({:.1})",
                s.x + s.w
            );
        }
        // The crossing of the frame's top border is an explicit connection
        // point.
        assert!(
            e.points
                .iter()
                .any(|&(x, y)| (y - s.y).abs() < 1e-2 && x > s.x && x < s.x + s.w),
            "no connection point on the frame's top border: {:?}",
            e.points
        );
        // The final approach is horizontal into t's right side.
        let (a, b) = (e.points[e.points.len() - 2], *e.points.last().unwrap());
        assert!(
            (a.1 - b.1).abs() < 1e-2 && a.0 > b.0,
            "final approach {a:?}->{b:?} not horizontal from the right"
        );
        assert_all_finite(&l);
    }

    #[test]
    fn forced_side_cross_boundary_edge_navigates_between_sibling_groups() {
        // M14 regression (the cluster shape): `user --> lb` forced into lb's
        // right side, with a sibling group stacked beside lb's inside the
        // outer frame. The edge descends in the corridor between the two
        // groups and hooks left into lb — instead of swinging around the
        // outer group's right border and cutting back across the sibling's
        // frames and under its nodes.
        let (_d, l) = lay_final(
            r#"diagram top-down
user "User"
group left-right
  group "App"
    lb "LB"
  end
  group "Shared"
    n "N"
  end
end
lb --> n
user -- to="right" --> lb
"#,
        );
        let lb = node_rect(&l, "lb");
        let n = node_rect(&l, "n");
        let e = edge_path(&l, "user", "lb");
        let pts = &e.points;
        assert!(on_side(lb, *pts.last().unwrap(), "right"));
        assert!(is_orthogonal(pts));
        // Groups in declaration order: [0] outer, [1] App, [2] Shared.
        let outer = &l.groups[0];
        let app = &l.groups[1];
        let shared = &l.groups[2];
        for &(x, _) in pts.iter() {
            assert!(
                x < outer.x + outer.w - 1.0,
                "point x {x} wrapped around the outer frame's right border"
            );
        }
        for w in pts.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], (shared.x, shared.y, shared.w, shared.h)),
                "segment {w:?} crosses the sibling group's frame"
            );
            assert!(
                !segment_intersects_rect(w[0], w[1], (n.x, n.y, n.w, n.h)),
                "segment {w:?} passes through the sibling's node"
            );
        }
        // Both group crossings are explicit connection points: the outer
        // frame's top border (the natural entry) and the App frame's right
        // border at the port's height (the "left into the node" turn).
        assert!(
            pts.iter()
                .any(|&(x, y)| (y - outer.y).abs() < 1e-2 && x > outer.x && x < outer.x + outer.w),
            "no connection point on the outer frame's top border: {pts:?}"
        );
        assert!(
            pts.iter().any(|&(x, y)| {
                (x - (app.x + app.w)).abs() < 1e-2 && y > app.y && y < app.y + app.h
            }),
            "no connection point on the App frame's right border: {pts:?}"
        );
        // The final approach is horizontal into lb's right side.
        let (a, b) = (pts[pts.len() - 2], *pts.last().unwrap());
        assert!(
            (a.1 - b.1).abs() < 1e-2 && a.0 > b.0,
            "final approach {a:?}->{b:?} not horizontal from the right"
        );
    }

    #[test]
    fn contradictory_forced_side_on_cross_boundary_edge_wraps_the_node() {
        // `src --> t` forced into t's BOTTOM side, facing away from the
        // source (M14): the frame is still entered at its natural (top)
        // crossing, and the stub wraps around t — beside and under it — into
        // the bottom port, never through t or its sibling below.
        let (_d, l) = lay_final(
            r#"diagram top-down
src
group "G"
t "Target"
sib "Sibling"
t --> sib
end
src -- to="bottom" --> t
"#,
        );
        let t = node_rect(&l, "t");
        let sib = node_rect(&l, "sib");
        let e = edge_path(&l, "src", "t");
        assert!(on_side(t, *e.points.last().unwrap(), "bottom"));
        assert!(is_orthogonal(&e.points));
        for w in e.points.windows(2) {
            for r in [(t.x, t.y, t.w, t.h), (sib.x, sib.y, sib.w, sib.h)] {
                assert!(
                    !segment_intersects_rect(w[0], w[1], r),
                    "segment {w:?} passes through a node"
                );
            }
        }
        assert_all_finite(&l);
    }

    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_sides_for_inspection() {
        let (_d, l) = lay_final(include_str!("../examples/sides.dgmr"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap()
                .then(a.x.partial_cmp(&b.x).unwrap())
        });
        for n in &nodes {
            println!(
                "  {:<10} x={:7.1} y={:7.1} w={:5.1} h={:5.1}",
                n.id, n.x, n.y, n.w, n.h
            );
        }
        for e in &l.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  {:<8} -> {:<8} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn forced_sides_keep_m9_invariants_on_the_example() {
        let (d, l) = lay_final(include_str!("../examples/sides.dgmr"));
        assert_eq!(d.edges.len(), l.edges.len());
        assert_all_finite(&l);
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
        }
        assert_no_edge_overlaps_or_node_passage(&d, &l);
    }

    // ================= M13 — label placement & parallel spacing =================

    /// The knockout rect the renderer draws for edge `i`'s label.
    fn label_knockout_of(d: &crate::ast::Diagram, l: &Layout, i: usize) -> (f32, f32, f32, f32) {
        let label = d.edges[i].label.as_deref().expect("edge has a label");
        let (ax, ay) = l.edges[i].label_at.expect("labeled edge has an anchor");
        label_rect((ax, ay), label_box(label))
    }

    /// Assert the M13 label guarantees on a laid-out diagram: exactly the
    /// labeled edges carry anchors; each knockout touches its own line (the
    /// read-as-broken break) while covering no other edge's line, no other
    /// label, no node interior, no rendered arrowhead, and no group title.
    /// Frame borders are deliberately absent (M13.1): a label may cross a
    /// group's outline.
    fn assert_labels_resolved(d: &crate::ast::Diagram, l: &Layout) {
        for (e, p) in d.edges.iter().zip(&l.edges) {
            assert_eq!(
                p.label_at.is_some(),
                e.label.is_some(),
                "edge {}->{} label/anchor mismatch",
                e.from,
                e.to
            );
        }
        let mut placed: Vec<PlacedLabel> = Vec::new();
        for i in 0..d.edges.len() {
            if d.edges[i].label.is_none() {
                continue;
            }
            let p = &l.edges[i];
            let r = label_knockout_of(d, l, i);
            // (1) The knockout touches its own line — on-line and graze
            // placements alike keep the break behind the text.
            assert!(
                (0..p.points.len().saturating_sub(1)).any(|k| segment_intersects_rect(
                    p.points[k],
                    p.points[k + 1],
                    r
                )),
                "label {:?} is detached from its edge {}->{}",
                d.edges[i].label,
                p.from,
                p.to
            );
            // (2) It covers no other edge's line — the defect M13 eliminates.
            let hit = inflate(r, LABEL_HIT_PAD);
            for (j, q) in l.edges.iter().enumerate() {
                if j == i {
                    continue;
                }
                for w in q.points.windows(2) {
                    let covered = segment_covered_len(w[0], w[1], hit);
                    assert!(
                        covered < 0.5,
                        "label {:?} covers {:.1}px of edge {}->{}",
                        d.edges[i].label,
                        covered,
                        q.from,
                        q.to
                    );
                }
            }
            // (3) It overlaps no other label's knockout.
            for &(j, or_) in &placed {
                assert!(
                    rect_overlap_area(r, or_) <= 0.0,
                    "labels {:?} (#{i}) and {:?} (#{j}) overlap",
                    d.edges[i].label,
                    d.edges[j].label
                );
            }
            // (4) It covers no node interior.
            for n in &l.nodes {
                assert!(
                    rect_overlap_area(r, (n.x, n.y, n.w, n.h)) <= 0.0,
                    "label {:?} covers node {}",
                    d.edges[i].label,
                    n.id
                );
            }
            // (5) It covers no rendered arrowhead and no group title. (Frame
            // borders are not obstacles — a label may cross a group outline,
            // M13.1.)
            for q in l.edges.iter() {
                if let Some(a) = arrowhead_rect(&q.points) {
                    assert!(
                        rect_overlap_area(r, a) <= 0.0,
                        "label {:?} (edge {}->{}) covers the arrowhead of edge {}->{}",
                        d.edges[i].label,
                        p.from,
                        p.to,
                        q.from,
                        q.to
                    );
                }
            }
            for s in &l.groups {
                if let Some(t) = d.groups[s.index].title.as_deref() {
                    let tr = title_text_rect((s.x, s.y, s.w, s.h), t);
                    assert!(
                        rect_overlap_area(r, tr) <= 0.0,
                        "label {:?} covers the title {t:?} of frame #{}",
                        d.edges[i].label,
                        s.index
                    );
                }
            }
            placed.push((i, r));
        }
    }

    // ---- M20.2: the whole-path search as the direct-edge router ----

    /// The search is the live router for unforced direct edges (M20.2), and
    /// it re-derives the flat engine's shapes rather than inventing new ones:
    /// a clean diagonal still gets the familiar mid-gap Z, with approach
    /// stubs long enough to read under the arrowhead.
    #[test]
    fn whole_path_search_routes_direct_edges() {
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\nb-->d\nc-->d\n");
        for e in &l.edges {
            // The classic Z: out, across at the gap's midpoint, in.
            assert_eq!(e.points.len(), 4, "{}->{}: {:?}", e.from, e.to, e.points);
            let (a, b) = (e.points[0], e.points[1]);
            let (c, d) = (e.points[2], e.points[3]);
            assert!((a.0 - b.0).abs() < 1e-2, "first leg must be vertical");
            assert!((c.0 - d.0).abs() < 1e-2, "last leg must be vertical");
            assert!((b.1 - c.1).abs() < 1e-2, "the cross-run is horizontal");
            // Both terminal legs clear the arrowhead's back-extent (10).
            assert!(b.1 - a.1 >= 10.0, "start stub {} too short", b.1 - a.1);
            assert!(d.1 - c.1 >= 10.0, "end stub {} too short", d.1 - c.1);
        }
    }

    /// The search jogs at the flat engine's own row (the flow midpoint), not
    /// at whichever track the heap reaches first: without the seeded jog rows
    /// every row ties on length and the tie lands one track gap off the
    /// target — an 8 px stub that disappears under the arrowhead.
    #[test]
    fn whole_path_search_jogs_at_the_flat_engines_row() {
        let (_d, l) = lay_final("diagram top-down\na-->b\na-->c\n");
        let p = &edge_path(&l, "a", "b").points;
        assert_eq!(p.len(), 4, "{p:?}");
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        let want = ((a.y + a.h) + b.y) / 2.0;
        assert!(
            (p[1].1 - want).abs() < 1e-2,
            "jog row {} is not the gap midpoint {want}",
            p[1].1
        );
    }

    /// A forced side keeps M11/M14 semantics: the port lands on the
    /// requested side and the edge is routed by the M11 ladder, whose escape
    /// geometry the search's free-choice port model would overwrite. The
    /// search is therefore scoped to *unforced* direct edges until M20.4
    /// re-grounds port selection.
    #[test]
    fn whole_path_search_leaves_forced_sides_to_the_ladder() {
        let (d, l) = lay_final(include_str!("../examples/sides.dgmr"));
        // `sides.dgmr` declares `api -> db` twice; the second is the forced
        // `from="top"` one.
        let ei = d
            .edges
            .iter()
            .position(|e| {
                e.from == "api" && e.to == "db" && e.from_side == Some(crate::ast::EdgeSide::Top)
            })
            .expect("the forced from=top api->db edge");
        let p = &l.edges[ei].points;
        let a = node_rect(&l, "api");
        // The forced port is on the node's top edge and the route leaves it
        // upward before turning — the M11 shape, untouched by the search.
        assert!((p[0].1 - a.y).abs() < 1e-2, "port not on the forced side");
        assert!(p[1].1 < p[0].1, "the port must escape outward");
    }





    // ============ M20.4 — frame-crossing fan ==========================

    /// The M20.4 pre-pass: edges entering one ancestor frame side from
    /// distinct sources take distinct crossing coordinates instead of
    /// collapsing onto the side's cheapest row. This is the plan's "Pre-pass
    /// 1, reframed": the frame sides the whole-path search crosses get the
    /// same fan [`separate_ports`] gives a shared node side.
    ///
    /// The defect it closes is measured on the repair loop: before M20.4 the
    /// `cluster` sample's `lb -> n*` edges all landed on one crossing column,
    /// the repair pass detected the collinear ride, and the loser was pushed
    /// all the way around the frames (678.6 px of extra route — pinned by
    /// `repair_loop_has_nothing_to_negotiate_on_the_cluster_sample`).
    #[test]
    fn edges_into_one_frame_side_cross_at_distinct_columns() {
        let (_d, l) = lay_final(
            "diagram top-down\n\
             s1 \"S1\"\n\
             s2 \"S2\"\n\
             group top-down \"G\"\n\
             a \"A\"\n\
             b \"B\"\n\
             end\n\
             s1 --> a\n\
             s2 --> b\n",
        );
        let g = sub_rect(&l, 0);
        // Check every frame side: the crossings through one side must be
        // FAN_SEP apart. (Which side an edge enters by is the search's
        // choice, so the assertion is per side rather than per edge.)
        for (sname, coord, horiz) in [
            ("top", g.y, true),
            ("bottom", g.y + g.h, true),
            ("left", g.x, false),
            ("right", g.x + g.w, false),
        ] {
            let crosses = frame_side_crossings(&l, g, sname, coord, horiz);
            for w in crosses.windows(2) {
                assert!(
                    w[1] - w[0] >= FAN_SEP - 1e-2,
                    "{sname} crossings collapsed: {crosses:?}"
                );
            }
        }
        // Both edges still land on their targets' boundaries.
        for e in &l.edges {
            let t = node_rect(&l, &e.to);
            assert!(
                on_boundary((t.x, t.y, t.w, t.h), *e.points.last().unwrap()),
                "{}->{} misses its target",
                e.from,
                e.to
            );
        }
    }

    /// The crossing coordinates of every edge through one frame side, sorted
    /// and deduped. `(side name, the side's fixed coordinate, whether the
    /// side is horizontal)`.
    fn frame_side_crossings(
        l: &Layout,
        g: &GroupRect,
        _sname: &str,
        coord: f32,
        horiz: bool,
    ) -> Vec<f32> {
        let mut crosses: Vec<f32> = Vec::new();
        for e in &l.edges {
            for w in e.points.windows(2) {
                let (p0, p1) = (w[0], w[1]);
                if horiz {
                    // A horizontal side (top/bottom) is crossed by a vertical
                    // segment at an x inside the side's span, with the segment
                    // spanning the side's coordinate.
                    if (p0.0 - p1.0).abs() > 1e-3
                        || p0.0 < g.x - 1e-2
                        || p0.0 > g.x + g.w + 1e-2
                    {
                        continue;
                    }
                    if p0.1.min(p1.1) <= coord + 1e-2 && p0.1.max(p1.1) >= coord - 1e-2 {
                        crosses.push(p0.0);
                    }
                } else {
                    if (p0.1 - p1.1).abs() > 1e-3
                        || p0.1 < g.y - 1e-2
                        || p0.1 > g.y + g.h + 1e-2
                    {
                        continue;
                    }
                    if p0.0.min(p1.0) <= coord + 1e-2 && p0.0.max(p1.0) >= coord - 1e-2 {
                        crosses.push(p0.1);
                    }
                }
            }
        }
        crosses.sort_by(f32::total_cmp);
        crosses.dedup_by(|a, b| (*a - *b).abs() < 1e-2);
        crosses
    }

    /// The fan is measured *per frame side*: edges entering different sides
    /// of one frame (or different frames) do not constrain each other.
    #[test]
    fn frame_crossing_fan_is_per_side() {
        // `s` feeds `a` (top of the inner group) and `b` sits in a *sibling*
        // group — two unrelated frames, so no fan applies and neither edge
        // may be pushed off its own natural column.
        let (_d, l) = lay_final(
            "diagram top-down\n\
             s \"S\"\n\
             group top-down \"Left\"\n\
             a \"A\"\n\
             end\n\
             group top-down \"Right\"\n\
             b \"B\"\n\
             end\n\
             s --> a\n\
             s --> b\n",
        );
        // Both frames are unrelated to one of the edges, so each edge has
        // exactly one ancestor frame — no shared side, no fan.
        for e in &l.edges {
            assert!(is_orthogonal(&e.points), "{}->{} not orthogonal", e.from, e.to);
        }
        assert_all_finite(&l);
    }

    /// The crossing fan must not break the phase-1 permutation invariance
    /// (M17): the pre-pass sorts by a content key (endpoint ids), so
    /// permuting the edges' declaration order permutes nothing.
    #[test]
    fn frame_crossing_fan_is_permutation_invariant() {
        let head = "diagram top-down\n\
                    s1 \"S1\"\n\
                    s2 \"S2\"\n\
                    s3 \"S3\"\n\
                    group top-down \"G\"\n\
                    a \"A\"\n\
                    b \"B\"\n\
                    c \"C\"\n\
                    end\n";
        let edges_a = ["s1 --> a", "s2 --> b", "s3 --> c", "s1 --> b"];
        let mut edges_b = edges_a;
        edges_b.reverse();
        let build = |edges: &[&str]| {
            let mut src = head.to_string();
            for e in edges {
                src.push_str(e);
                src.push('\n');
            }
            let raw = parser::parse_diagram(&src).expect("parse");
            resolve::resolve(&raw).expect("resolve")
        };
        let la = layout_phase1(&build(&edges_a));
        let lb = layout_phase1(&build(&edges_b));
        for e in &la.edges {
            let other = lb
                .edges
                .iter()
                .find(|o| o.from == e.from && o.to == e.to)
                .unwrap_or_else(|| panic!("edge {}->{} missing after permutation", e.from, e.to));
            assert_eq!(
                e.points, other.points,
                "{}->{} moved under edge permutation",
                e.from, e.to
            );
        }
    }

    /// A frame-crossing fan must not collapse *when the sources are outside
    /// the frame's side span* (the case a naive clamp would break): sources
    /// fanning in from well left of the frame still get distinct crossings on
    /// whichever side they enter by.
    #[test]
    fn frame_crossing_fan_spreads_out_of_span_sources() {
        let (_d, l) = lay_final(
            "diagram top-down\n\
             s1 \"S1\"\n\
             s2 \"S2\"\n\
             s3 \"S3\"\n\
             group top-down \"G\"\n\
             a \"A\"\n\
             end\n\
             s1 --> a\n\
             s2 --> a\n\
             s3 --> a\n",
        );
        let g = sub_rect(&l, 0);
        for (sname, coord, horiz) in [
            ("top", g.y, true),
            ("bottom", g.y + g.h, true),
            ("left", g.x, false),
            ("right", g.x + g.w, false),
        ] {
            let crosses = frame_side_crossings(&l, g, sname, coord, horiz);
            for w in crosses.windows(2) {
                assert!(
                    w[1] - w[0] >= FAN_SEP - 1e-2,
                    "{sname} crossings closer than FAN_SEP: {crosses:?}"
                );
            }
        }
        // Every edge still reaches its target on the boundary.
        let a = node_rect(&l, "a");
        for e in &l.edges {
            assert!(
                on_boundary((a.x, a.y, a.w, a.h), *e.points.last().unwrap()),
                "{}->{} misses a's boundary",
                e.from,
                e.to
            );
        }
    }

    /// The M20.4 fan's contract on the shape it exists for: several edges
    /// from outside one nested frame fanning into a single target. The search
    /// alone sends each edge to its own cheapest row, so two of them approach
    /// the same frame border on nearly the same column and the repair loop has
    /// to push one around. The pre-pass seats them on distinct columns, so
    /// every crossing through each frame side is at least [`FAN_SEP`] apart.
    ///
    /// `frame_crossing_fan_is_what_seats_the_columns` below pins that this is
    /// the pre-pass's doing, not a coincidence of the cost model.
    #[test]
    fn frame_crossing_fan_seats_nested_frame_crossings() {
        let src = "diagram top-down\n\
                   x0 \"External 0\"\n\
                   x1 \"External 1\"\n\
                   x2 \"External 2\"\n\
                   x3 \"External 3\"\n\
                   x4 \"External 4\"\n\
                   group top-down \"Outer\"\n\
                   group top-down \"Inner\"\n\
                   t \"Target\"\n\
                   end\n\
                   end\n\
                   x0 --> t\n\
                   x1 --> t\n\
                   x2 --> t\n\
                   x3 --> t\n\
                   x4 --> t\n";
        let (_d, l) = lay_final(src);
        // Every frame side of every frame: crossings >= FAN_SEP apart.
        for g in &l.groups {
            for (sname, coord, horiz) in [
                ("top", g.y, true),
                ("bottom", g.y + g.h, true),
                ("left", g.x, false),
                ("right", g.x + g.w, false),
            ] {
                let crosses = frame_side_crossings(&l, g, sname, coord, horiz);
                for w in crosses.windows(2) {
                    assert!(
                        w[1] - w[0] >= FAN_SEP - 1e-2,
                        "frame #{} {sname} crossings collapsed: {crosses:?}",
                        g.index
                    );
                }
            }
        }
        // The fan leaves the repair loop nothing to negotiate: phase 1 and
        // the final routes are the same length. (Without the pre-pass this
        // fixture's final routes are 140.4 px longer — the repair loop
        // pushing two of the five edges around the frames — which is the
        // regression this pins.)
        let raw = parser::parse_diagram(src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let total = |l: &Layout| -> f32 {
            l.edges
                .iter()
                .map(|e| {
                    e.points
                        .windows(2)
                        .map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs())
                        .sum::<f32>()
                })
                .sum()
        };
        let (p1, pf) = (total(&layout_phase1(&d)), total(&l));
        assert!(
            pf - p1 <= 20.0,
            "the repair loop added {:.1} px (phase 1 {p1:.1} -> final {pf:.1}): \
             a frame-crossing collapse is back",
            pf - p1
        );

        // Every edge still reaches the target on its boundary, and none cuts
        // a node.
        let t = node_rect(&l, "t");
        for e in &l.edges {
            assert!(
                on_boundary((t.x, t.y, t.w, t.h), *e.points.last().unwrap()),
                "{}->{} misses t",
                e.from,
                e.to
            );
            for w in e.points.windows(2) {
                for n in &l.nodes {
                    assert!(
                        !segment_intersects_rect(w[0], w[1], (n.x, n.y, n.w, n.h)),
                        "{}->{} cuts node {}",
                        e.from,
                        e.to,
                        n.id
                    );
                }
            }
        }
    }

    /// The M20.4 dense-stress re-baseline: many edges fanning into one frame
    /// — the shape the frame-crossing fan exists for. Each edge gets its own
    /// source port and its own target, so the fan (not the port separation)
    /// is what has to keep the crossings apart: phase 1 must already be
    /// conflict-free, and the repair loop must have nothing to move.
    #[test]
    fn dense_fan_into_one_frame_needs_no_repair() {
        let mut src = String::from("diagram top-down\n");
        let n = 8;
        for i in 0..n {
            src.push_str(&format!("s{i} \"Source {i}\"\n"));
        }
        src.push_str("group top-down \"Sink\"\n");
        for i in 0..n {
            src.push_str(&format!("t{i} \"Target {i}\"\n"));
        }
        src.push_str("end\n");
        // One source per target: n edges crossing one frame's border, each
        // from a different column.
        for i in 0..n {
            src.push_str(&format!("s{i} --> t{i}\n"));
        }
        let raw = parser::parse_diagram(&src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let l1 = layout_phase1(&d);
        let lf = layout(&d);
        assert_all_finite(&lf);
        assert_no_edge_overlaps_or_node_passage(&d, &lf);
        for e in &lf.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
            let t = node_rect(&lf, &e.to);
            assert!(
                on_boundary((t.x, t.y, t.w, t.h), *e.points.last().unwrap()),
                "{}->{} misses its target",
                e.from,
                e.to
            );
        }
        // Phase 1 must already be conflict-free (the fan's whole point): no
        // two phase-1 polylines may lay on top of each other.
        for i in 0..l1.edges.len() {
            for j in (i + 1)..l1.edges.len() {
                let (ei, ej) = (&l1.edges[i], &l1.edges[j]);
                for a in ei.points.windows(2) {
                    for b in ej.points.windows(2) {
                        assert!(
                            !segments_overlap(a[0], a[1], b[0], b[1]),
                            "phase-1 overlap {}->{} / {}->{}",
                            ei.from,
                            ei.to,
                            ej.from,
                            ej.to
                        );
                    }
                }
            }
        }
        // And the repair loop does not have to move anything.
        let total = |l: &Layout| -> f32 {
            l.edges
                .iter()
                .map(|e| {
                    e.points
                        .windows(2)
                        .map(|w| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs())
                        .sum::<f32>()
                })
                .sum()
        };
        assert!(
            (total(&lf) - total(&l1)).abs() < 1e-2,
            "the repair loop moved routes on a fan the pre-pass should have seated \
             (phase 1 {:.1} -> final {:.1})",
            total(&l1),
            total(&lf)
        );
    }

    #[test]
    fn labels_are_resolved_on_the_samples() {
        for src in [
            include_str!("../examples/finance.dgmr"),
            include_str!("../examples/infra.dgmr"),
            include_str!("../examples/subdirection.dgmr"),
            include_str!("../examples/sides.dgmr"),
        ] {
            let (d, l) = lay_final(src);
            assert_labels_resolved(&d, &l);
        }
    }

    #[test]
    fn label_placement_is_deterministic() {
        let src = include_str!("../examples/finance.dgmr");
        let (d1, l1) = lay_final(src);
        let (d2, l2) = lay_final(src);
        let a1: Vec<_> = d1
            .edges
            .iter()
            .zip(&l1.edges)
            .map(|(e, p)| (e.label.clone(), p.label_at))
            .collect();
        let a2: Vec<_> = d2
            .edges
            .iter()
            .zip(&l2.edges)
            .map(|(e, p)| (e.label.clone(), p.label_at))
            .collect();
        assert_eq!(a1, a2);
    }

    #[test]
    fn clear_straight_edge_label_sits_at_its_midpoint() {
        let (_d, l) = lay_final("diagram top-down\na -- \"sync\" --> b\n");
        let (ax, ay) = l.edges[0].label_at.expect("anchor");
        let p = &l.edges[0].points;
        assert_eq!(p.len(), 2, "straight edge");
        assert!(
            (ax - (p[0].0 + p[1].0) / 2.0).abs() < 1e-3,
            "x off midpoint: {ax}"
        );
        assert!(
            (ay - (p[0].1 + p[1].1) / 2.0).abs() < 1e-3,
            "y off midpoint: {ay}"
        );
    }

    #[test]
    fn unobstructed_label_sits_on_the_longest_run_midpoint() {
        // The classic look kept: when nothing collides, the anchor is the
        // longest segment's midpoint — the pre-M13 policy.
        let labels = [Some("hello world"), None];
        let paths = vec![
            vec![(0.0, 0.0), (0.0, 10.0), (40.0, 10.0), (40.0, 20.0)],
            Vec::new(),
        ];
        let world = LabelWorld {
            nodes: Vec::new(),
            frames: Vec::new(),
            arrows: Vec::new(),
        };
        let anchors = place_labels(&labels, &paths, &world);
        assert_eq!(anchors[0], Some((20.0, 10.0)));
        assert_eq!(anchors[1], None);
    }

    #[test]
    fn label_clears_the_arrowhead_of_the_final_segment() {
        // The edge bends right just before its target, so the longest run is
        // the final horizontal one — whose end carries the rendered
        // arrowhead. The knockout must back off along the run (or escape to
        // another candidate) rather than sit on the arrowhead that
        // terminates the line.
        let labels = [Some("sync"), None];
        let path = vec![(0.0, 0.0), (0.0, 30.0), (60.0, 30.0)];
        let world = LabelWorld {
            nodes: Vec::new(),
            frames: Vec::new(),
            arrows: arrowhead_rect(&path).into_iter().collect(),
        };
        let anchors = place_labels(&labels, std::slice::from_ref(&path), &world);
        let (ax, ay) = anchors[0].expect("anchor");
        let r = label_rect((ax, ay), label_box("sync"));
        let a = arrowhead_rect(&path).expect("arrowhead rect");
        assert!(
            rect_overlap_area(r, a) <= 0.0,
            "label knockout {r:?} covers its own arrowhead {a:?}"
        );
    }

    #[test]
    fn arrowhead_rect_bounds_the_rendered_marker() {
        // The rect covers `ARROW_LEN` back from the tip along the final
        // segment, `ARROW_HALF_W` perpendicular each side, padded — matching
        // the renderer's marker geometry.
        let path = vec![(0.0, 0.0), (0.0, 100.0)];
        let a = arrowhead_rect(&path).expect("arrowhead rect");
        assert_eq!(
            a,
            (
                0.0 - ARROW_HALF_W - ARROW_PAD,
                100.0 - ARROW_LEN - ARROW_PAD,
                2.0 * ARROW_HALF_W + 2.0 * ARROW_PAD,
                ARROW_LEN + 2.0 * ARROW_PAD
            )
        );
        assert!(arrowhead_rect(&[(0.0, 0.0)]).is_none());
    }

    #[test]
    fn degenerate_polyline_falls_back_to_the_old_anchor() {
        let labels = [Some("x"), Some("y")];
        let world = LabelWorld {
            nodes: Vec::new(),
            frames: Vec::new(),
            arrows: Vec::new(),
        };
        let anchors = place_labels(&labels, &[vec![(3.0, 4.0)], Vec::new()], &world);
        assert_eq!(anchors[0], Some((3.0, 4.0)));
        assert_eq!(anchors[1], Some((0.0, 0.0)));
    }

    #[test]
    fn refinement_pass_frees_a_label_crowded_by_a_later_one() {
        // L1 has a long free vertical run with several good spots; L2 sits on
        // a short stub beside the best one and is forced onto it (its graze
        // escapes are blocked by a third line and by L1's own line). The
        // greedy pass — L1 first, blind to L2 — strands L1's knockout under
        // L2's; the refinement pass moves L1 to its next-best spot. Geometry
        // (derived from the real knockout sizes):
        //
        //   L1: vertical run at x1 (y 0..200); its midpoint knockout spans
        //       [x1 - w1/2, x1 + w1/2].
        //   L2: 8px stub at x2 = x1 - w2/2 - w1/4 — far enough that L2's
        //       on-line knockout stops short of L1's line, near enough that it
        //       still reaches into L1's midpoint knockout; L2's right graze
        //       covers L1's line, its left graze a third line at
        //       x3 = x2 - 0.75 * w2.
        let l1 = "LLLLL";
        let l2 = "HHHHHHHH";
        let (w1, _h1) = label_box(l1);
        let (w2, _h2) = label_box(l2);
        let x1 = 200.0_f32;
        let x2 = x1 - w2 / 2.0 - w1 / 4.0;
        let x3 = x2 - 0.75 * w2;
        let labels = [Some(l1), Some(l2), None];
        let paths = vec![
            vec![(x1, 0.0), (x1, 200.0)],
            vec![(x2, 96.0), (x2, 104.0)],
            vec![(x3, 85.0), (x3, 115.0)],
        ];
        let world = LabelWorld {
            nodes: Vec::new(),
            frames: Vec::new(),
            arrows: Vec::new(),
        };
        let anchors = place_labels(&labels, &paths, &world);
        let a1 = anchors[0].expect("L1 anchor");
        let a2 = anchors[1].expect("L2 anchor");
        // L2 keeps its one clear spot (mid-stub)...
        assert!(
            (a2.0 - x2).abs() < 1e-3,
            "L2 should stay on its stub: {a2:?}"
        );
        assert!(
            (a2.1 - 100.0).abs() < 4.0,
            "L2 should stay mid-stub: {a2:?}"
        );
        // ...while L1, crowded there by L2's knockout, moves off the midpoint.
        assert!(a1.1 < 80.0, "L1 should move clear of L2: {a1:?}");
        assert_eq!(
            rect_overlap_area(label_rect(a1, label_box(l1)), label_rect(a2, label_box(l2))),
            0.0,
            "refinement should leave the knockouts disjoint"
        );
    }

    #[test]
    fn label_pair_sep_grows_with_the_knockout_and_respects_its_bounds() {
        // Unlabeled pairs keep the base gap.
        assert_eq!(label_pair_sep(0.0, 0.0, FAN_SEP, None), FAN_SEP);
        // A labeled pair clears the wider knockout's half-extent + LABEL_CLEAR.
        assert_eq!(
            label_pair_sep(20.0, 0.0, FAN_SEP, None),
            20.0 / 2.0 + LABEL_CLEAR
        );
        assert_eq!(
            label_pair_sep(20.0, 30.0, FAN_SEP, None),
            30.0 / 2.0 + LABEL_CLEAR
        );
        // The fan bump is capped (a fan must fit real node sides); lanes are
        // not (their gap band clamps them).
        assert_eq!(
            label_pair_sep(200.0, 200.0, FAN_SEP, Some(FAN_LABEL_CAP)),
            FAN_LABEL_CAP
        );
        assert_eq!(
            label_pair_sep(200.0, 200.0, FAN_SEP, None),
            200.0 / 2.0 + LABEL_CLEAR
        );
        // The base floor wins where it already clears the label.
        assert_eq!(label_pair_sep(4.0, 4.0, 30.0, None), 30.0);
    }

    #[test]
    fn labeled_fan_ports_spread_wider_than_unlabeled() {
        // Three edges sharing one node side, wanting nearly the same port
        // cross. Unlabeled they keep their desired crosses; with labels they
        // fan wider so an on-line knockout cannot reach the neighbor's line.
        let mk = |cover: f32| -> Vec<PortEnd> {
            [100.0, 110.0, 120.0]
                .iter()
                .enumerate()
                .map(|(k, &x)| PortEnd {
                    edge: k,
                    is_from: true,
                    node_idx: 0,
                    node_rect: (0.0, 0.0, 300.0, 40.0),
                    side: Side::Top,
                    other_cross: x,
                    cover,
                    frame: false,
                    from_id: format!("f{k}"),
                    to_id: format!("t{k}"),
                })
                .collect()
        };
        let sorted = |m: &std::collections::HashMap<(usize, bool), f32>| -> Vec<f32> {
            let mut v: Vec<f32> = (0..3).map(|k| m[&(k, true)]).collect();
            v.sort_by(|a, b| a.total_cmp(b));
            v
        };
        let plain = sorted(&separate_ports(&mk(0.0)));
        let labeled = sorted(&separate_ports(&mk(45.0)));
        assert!(
            plain[1] - plain[0] < 15.0 && plain[2] - plain[1] < 15.0,
            "unlabeled fan keeps its tight desired crosses: {plain:?}"
        );
        // 45-wide knockouts: at least FAN_LABEL_CAP (>= 45/2 + LABEL_CLEAR)
        // apart where the side has the room (it has: 300px wide).
        assert!(
            labeled[1] - labeled[0] >= FAN_LABEL_CAP - 1e-3
                && labeled[2] - labeled[1] >= FAN_LABEL_CAP - 1e-3,
            "labeled fan must spread: {labeled:?}"
        );
    }

    #[test]
    fn labeled_lanes_take_their_room_before_the_slack() {
        // Three parallel cross-boundary edges in one gap: the first carries a
        // wide (60px) two-line label, the others none. The labeled gap grows
        // to clear its knockout (60/2 + LABEL_CLEAR) and the slack is shared
        // on top — while an unlabeled group still spreads perfectly evenly
        // (the pre-M13 behaviour).
        let lanes = |covers: [f32; 3]| -> Vec<f32> {
            let edges: Vec<LaneEdge> = covers
                .iter()
                .enumerate()
                .map(|(k, &c)| LaneEdge {
                    edge: k,
                    rep_from: ItemRef::Node(0),
                    from_side: Side::Bottom,
                    rep_to: ItemRef::Node(1),
                    from_flow: 0.0,
                    to_flow: 100.0,
                    from_cross: k as f32,
                    to_cross: k as f32,
                    axis: FlowAxis::Vertical,
                    cover: c,
                    from_id: format!("f{k}"),
                    to_id: format!("t{k}"),
                    obstacles: Vec::new(),
                })
                .collect();
            let m = assign_lanes(&edges);
            (0..3).map(|k| m[&k]).collect()
        };
        let plain = lanes([0.0, 0.0, 0.0]);
        let labeled = lanes([60.0, 0.0, 0.0]);
        assert!(
            (plain[1] - plain[0] - (plain[2] - plain[1])).abs() < 1e-3,
            "unlabeled lanes spread evenly: {plain:?}"
        );
        assert!(
            labeled[1] - labeled[0] >= 60.0 / 2.0 + LABEL_CLEAR - 1e-3,
            "the labeled lane's gap must clear its knockout: {labeled:?}"
        );
        assert!(
            labeled[1] - labeled[0] > plain[1] - plain[0],
            "labels must widen their lane gap: {labeled:?} vs {plain:?}"
        );
        assert!(
            labeled[2] - labeled[1] >= FAN_SEP - 1e-3,
            "the unlabeled gap keeps its floor: {labeled:?}"
        );
    }

    // ============ Sealed-port fallback (post-M13) ============

    #[test]
    fn port_alternates_centre_then_corner_ports() {
        // A 100-wide bottom side, partner far right: the current (centre)
        // cross first, then the PORT_INSET corner ports, the partner-facing
        // one first.
        let alts = port_alternates((0.0, 0.0, 100.0, 40.0), Side::Bottom, 50.0, 500.0);
        assert_eq!(alts, vec![50.0, 93.0, 7.0]);
        // A partner to the left flips the corner order.
        let alts = port_alternates((0.0, 0.0, 100.0, 40.0), Side::Bottom, 50.0, -500.0);
        assert_eq!(alts, vec![50.0, 7.0, 93.0]);
        // Vertical sides are their y-span, same rule.
        let alts = port_alternates((0.0, 0.0, 40.0, 100.0), Side::Right, 50.0, 500.0);
        assert_eq!(alts, vec![50.0, 93.0, 7.0]);
        // No room to move: just the current coordinate.
        let alts = port_alternates((0.0, 0.0, 12.0, 40.0), Side::Bottom, 6.0, 500.0);
        assert_eq!(alts, vec![6.0]);
    }

    #[test]
    fn sealed_port_fallback_slides_a_lone_port_to_its_corner() {
        // A sealed escape pocket, miniaturised (the M11.5 failure): `w` sits
        // right under `a`'s bottom centre, so the centre-port drop pierces
        // `w` and `track_route` cannot leave the port at all (every first
        // move from it is blocked). The kept-unconditional Z is the only
        // "route" — and it cuts through `w`. Sliding `a`'s lone bottom port
        // to the partner-facing corner (PORT_INSET in) opens the corridor.
        let d = lay_final("diagram top-down\na \"A\"\nb \"B\"\nw \"W\"\na --> b\n").0;
        let mut id_index = std::collections::HashMap::new();
        id_index.insert("a", 0usize);
        id_index.insert("b", 1);
        id_index.insert("w", 2);
        let mut node_rect = std::collections::HashMap::new();
        node_rect.insert(0usize, (0.0, 0.0, 100.0, 40.0)); // a
        node_rect.insert(1, (200.0, 80.0, 100.0, 40.0)); // b
        node_rect.insert(2, (40.0, 45.0, 20.0, 30.0)); // w — seals a's escape
        let mut direct_pts = std::collections::HashMap::new();
        direct_pts.insert(0usize, vec![(50.0, 40.0), (250.0, 80.0)]);
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect,
            frame_rect: std::collections::HashMap::new(),
            direct_pts,
            level_items: std::collections::HashMap::new(),
        };
        // Context + the separation fill's single-edge case (ports at the
        // sides' centre) — what `assemble` builds before the routing phase.
        let mut ctx = edge_context(&geom, &[], 0);
        ctx.from_port = port_at_cross(
            ctx.from_rect,
            ctx.from_side,
            cross_of(center(ctx.from_rect), ctx.from_side),
        );
        ctx.to_port = port_at_cross(
            ctx.to_rect,
            ctx.to_side,
            cross_of(center(ctx.to_rect), ctx.to_side),
        );
        ctx.from_cross = cross_of(ctx.from_port, ctx.from_side);
        ctx.to_cross = cross_of(ctx.to_port, ctx.to_side);
        let mut world = Obstacles::empty();
        world.push_node((0.0, 0.0, 100.0, 40.0));
        world.push_node((200.0, 80.0, 100.0, 40.0));
        world.push_node((40.0, 45.0, 20.0, 30.0));
        let canvas = (400.0, 200.0);

        // The centre-port route is the defective terminal fallback.
        let bad = route_edge(&ctx, &world, &geom, canvas);
        assert!(!world.clear(&bad), "fixture must reproduce the sealed port");

        // The fallback slides `a`'s port to its partner-facing corner
        // (7 px in from the bottom-right) and routes clear; the target port
        // stays put (minimal deviation).
        let shared = std::collections::HashSet::new();
        let alt = port_fallback_route(&ctx, &world, &geom, canvas, &shared)
            .expect("a corner port routes clear");
        assert!(world.clear(&alt));
        assert_eq!(alt[0], (93.0, 40.0));
        assert_eq!(alt[alt.len() - 1], (250.0, 80.0));

        // A shared side is the fan's to place — the fallback declines and
        // the caller keeps its (defective) route.
        let shared: std::collections::HashSet<(usize, Side)> =
            [(0, Side::Bottom)].into_iter().collect();
        assert!(port_fallback_route(&ctx, &world, &geom, canvas, &shared).is_none());
    }

    /// Sweep a sample for edge segments cutting node interiors — the M11
    /// "routed around … never through a node" promise, checked without the
    /// bounded-last-resort carve-out.
    fn assert_no_edge_through_any_node(src: &str) {
        let (_d, l) = lay_final(src);
        for e in &l.edges {
            for w in e.points.windows(2) {
                for n in &l.nodes {
                    assert!(
                        !segment_intersects_rect(w[0], w[1], (n.x, n.y, n.w, n.h)),
                        "edge {}->{} segment {:?} cuts node {}",
                        e.from,
                        e.to,
                        w,
                        n.id
                    );
                }
            }
        }
    }

    #[test]
    fn example_edges_never_pass_through_nodes() {
        // The M11.5 verdict recorded `examples/sides.dgmr`'s
        // `api -- from="top" --> db` as the one known-broken case (its
        // ladder fell to the kept-unconditional Z, which drew through
        // `api`): a sealed port's escape pocket left the port unreachable.
        // The sealed-port fallback (a lone side's port slides to its corner
        // ports) removes the carve-out.
        assert_no_edge_through_any_node(include_str!("../examples/sides.dgmr"));
        assert_no_edge_through_any_node(include_str!("../examples/finance.dgmr"));
        assert_no_edge_through_any_node(include_str!("../examples/infra.dgmr"));
        assert_no_edge_through_any_node(include_str!("../examples/subdirection.dgmr"));
        assert_no_edge_through_any_node(include_str!("../examples/tencent.dgmr"));
        assert_no_edge_through_any_node(include_str!("../examples/cluster.dgmr"));
    }

    // ================= M16 — global routing pass =================

    #[test]
    fn stacked_group_cross_traffic_keeps_ports_reachable() {
        // The M15 known limitation, which the global pass now owns: a frame
        // whose members stack along its flow axis (edge-less members, M15
        // component packing) and receive cross-boundary edges from different
        // sources. Under the per-case regime a within-frame stub could cut a
        // stacked sibling and two sources' routes could coincide; here the
        // simultaneous pass plus negotiation must keep every invariant and
        // land every edge on its own target's boundary (a reachable port).
        let src = "diagram top-down\n\
                   s1 \"Source One\"\n\
                   s2 \"Source Two\"\n\
                   group top-down \"Stacked Targets\"\n\
                   m1 \"Target One\"\n\
                   m2 \"Target Two\"\n\
                   end\n\
                   s1 --> m1\n\
                   s2 --> m2\n";
        let (d, l) = lay_final(src);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        assert_no_edge_overlaps_or_node_passage(&d, &l);
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
            let target = node_rect(&l, &e.to);
            let last = *e.points.last().unwrap();
            assert!(
                on_boundary((target.x, target.y, target.w, target.h), last),
                "edge {}->{} does not reach its target's boundary: {last:?}",
                e.from,
                e.to
            );
            // And neither edge passes through any node at all (the stricter
            // sweep, including its own endpoints' interiors).
            for w in e.points.windows(2) {
                for n in &l.nodes {
                    assert!(
                        !segment_intersects_rect(w[0], w[1], (n.x, n.y, n.w, n.h)),
                        "edge {}->{} segment {w:?} cuts node {}",
                        e.from,
                        e.to,
                        n.id
                    );
                }
            }
        }
    }

    #[test]
    fn cross_source_lanes_in_one_gap_are_coordinated() {
        // Two edges from different sources share one gap (same source-side
        // flow and same target-side flow) — M9 grouped lanes per source, so
        // their jog lanes could park a few px apart and the one-pass regime
        // then arbitrated by declaration order. The global pass coordinates
        // the whole gap's lanes at once (simultaneous assignment): the two
        // cross runs sit at least [`FAN_SEP`] apart without any repair.
        let src = "diagram top-down\n\
                   u1 \"Source One\"\n\
                   u2 \"Source Two\"\n\
                   group top-down \"G\"\n\
                   a \"A\"\n\
                   b \"B\"\n\
                   end\n\
                   u1 --> a\n\
                   u2 --> b\n";
        let (_d, l) = lay_final(src);
        let lane_of = |id: &str| {
            let e = edge_path(
                &l,
                if id == "a" { "u1" } else { "u2" },
                if id == "a" { "a" } else { "b" },
            );
            // The cross run: the widest horizontal segment (top-down).
            e.points
                .windows(2)
                .filter(|w| (w[0].1 - w[1].1).abs() < 1e-3)
                .map(|w| w[0].1)
                .next_back()
                .unwrap_or_else(|| panic!("no horizontal run on {id}'s edge"))
        };
        let (la, lb) = (lane_of("a"), lane_of("b"));
        assert!(
            (la - lb).abs() >= FAN_SEP - 1e-3,
            "cross-source lanes {la} vs {lb} closer than FAN_SEP"
        );
    }

    #[test]
    fn repair_moves_an_invader_out_of_a_ports_pocket() {
        // Unit check of the repair scan's three violation kinds, on
        // hand-built routes: (a) a route through a node, (b) a collinear
        // rider on a peer, (c) an invader riding inside another edge's
        // reserved port pocket — the M11.5 sealed-pocket shape.
        let d =
            lay_final("diagram top-down\na \"A\"\nb \"B\"\nc \"C\"\nd \"D\"\na --> b\nc --> d\n").0;
        let mut id_index = std::collections::HashMap::new();
        for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
            id_index.insert(*id, i);
        }
        let mut node_rect = std::collections::HashMap::new();
        node_rect.insert(0usize, (0.0, 0.0, 100.0, 40.0)); // a
        node_rect.insert(1, (0.0, 200.0, 100.0, 40.0)); // b
        node_rect.insert(2, (200.0, 0.0, 100.0, 40.0)); // c
        node_rect.insert(3, (200.0, 200.0, 100.0, 40.0)); // d
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect,
            frame_rect: std::collections::HashMap::new(),
            direct_pts: std::collections::HashMap::new(),
            level_items: std::collections::HashMap::new(),
        };
        let ctxs: Vec<EdgeCtx> = (0..2)
            .map(|ei| {
                let mut v = edge_context(&geom, &[], ei);
                v.from_port = port_at_cross(
                    v.from_rect,
                    v.from_side,
                    cross_of(center(v.from_rect), v.from_side),
                );
                v.to_port =
                    port_at_cross(v.to_rect, v.to_side, cross_of(center(v.to_rect), v.to_side));
                v.from_cross = cross_of(v.from_port, v.from_side);
                v.to_cross = cross_of(v.to_port, v.to_side);
                v
            })
            .collect();
        // Edge 0's clean route: straight down a's centre, a long trunk whose
        // pockets only cover the ends (the swept escapes).
        let clean0 = vec![(50.0, 40.0), (50.0, 200.0)];
        // Edge 1's clean route: straight down c→d, clear of everything.
        let clean1 = vec![(250.0, 40.0), (250.0, 200.0)];
        let pockets = vec![pocket_rects(&clean0), pocket_rects(&clean1)];
        let scan =
            |paths: &[Option<Vec<(f32, f32)>>]| scan_violations(paths, &ctxs, &geom, &pockets);
        let paths = [Some(clean0.clone()), Some(clean1.clone())];
        assert!(scan(&paths).is_empty(), "clean routes flagged");

        // (a) edge 0's route cuts through b's interior (a hard defect).
        let paths = [
            Some(vec![(50.0, 40.0), (50.0, 220.0)]),
            Some(clean1.clone()),
        ];
        assert_eq!(scan(&paths).into_iter().collect::<Vec<_>>(), vec![0]);

        // (b) a mutual mid-trunk ride: edge 1's leg sits at edge 0's own x,
        // between the two pockets (neither endpoint's escape is involved) —
        // both negotiate.
        let paths = [
            Some(clean0.clone()),
            Some(vec![(50.0, 120.0), (50.0, 140.0)]),
        ];
        assert_eq!(scan(&paths).into_iter().collect::<Vec<_>>(), vec![0, 1]);

        // (c) edge 1's segment rides inside edge 0's source pocket (parallel
        // to the escape, within the pocket band) — the port has right-of-way:
        // the invader alone moves.
        let paths = [Some(clean0.clone()), Some(vec![(48.0, 45.0), (48.0, 60.0)])];
        assert_eq!(scan(&paths).into_iter().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn global_pass_stress_dense_compound_graph() {
        // A fixed LCG-generated dense compound graph (deterministic): 24
        // nodes in nested groups, 17 edges — far past what the samples
        // exercise. The global pass must keep every invariant: orthogonal,
        // no pass-through, no collinear overlap, every edge on its target's
        // boundary, all finite.
        let mut seed = 0x2545_f491_u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut src = String::from("diagram top-down\n");
        let n = 24;
        for i in 0..n {
            src.push_str(&format!("n{i} \"Node {i}\"\n"));
        }
        // Two sibling groups + one nested, members assigned round-robin.
        src.push_str("group left-right \"Outer\"\n");
        for i in 4..12 {
            src.push_str(&format!("n{i}\n"));
        }
        src.push_str("end\n");
        src.push_str("group top-down \"Inner\"\n");
        for i in 12..20 {
            src.push_str(&format!("n{i}\n"));
        }
        src.push_str("end\n");
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for _ in 0..30 {
            let a = rnd() as usize % n;
            let b = rnd() as usize % n;
            if a != b && !edges.contains(&(a, b)) {
                edges.push((a, b));
            }
        }
        // Keep the graph acyclic-ish for ranking sanity (feed-forward).
        edges.retain(|&(a, b)| a < b);
        for &(a, b) in &edges {
            src.push_str(&format!("n{a} --> n{b}\n"));
        }
        let (d, l) = lay(&src, false);
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        assert_no_edge_overlaps_or_node_passage(&d, &l);
        for e in &l.edges {
            assert!(
                is_orthogonal(&e.points),
                "{}->{} not orthogonal",
                e.from,
                e.to
            );
            let t = node_rect(&l, &e.to);
            let last = *e.points.last().unwrap();
            assert!(
                on_boundary((t.x, t.y, t.w, t.h), last),
                "{}->{} misses its target port: {last:?}",
                e.from,
                e.to
            );
        }
    }

    // ============ S19c — corridor coordination & seam bars ===================

    /// The M16 frame rep-crossing fan must be consulted by *unforced*
    /// cross-boundary edges: two edges entering the same frame side take
    /// their fanned crossings (distinct columns on the frame border), not
    /// one shared column — before S19c the dispatch only pushed the
    /// coupled candidate for an unforced edge, which reads the raw
    /// node-port crosses and never consults the fan, so two edges into one
    /// frame parked their approaches on one column and overlapped (the
    /// dense-stress row-fan defect).
    // ============ M17 — phase-1 permutation invariance ======================
    // M16 finding 2, enshrined: the routing pass's phase 1 is simultaneous —
    // every edge routes against the same world (nodes, frame borders,
    // reserved port pockets; no peer segments) — so its polylines must not
    // depend on the edges' declaration order. (The *final*, post-phase-2
    // routes deliberately may: the repair loop negotiates in index order —
    // the documented bounded residue in milestones.md.)
    #[test]
    fn phase1_polylines_do_not_depend_on_edge_order() {
        // All nodes are declared up front, so reordering the edge lines
        // cannot shift node declaration order (the order resolve()'s dedup
        // commits to). Every (from, to) pair is unique, so routes can be
        // matched across the two layouts by endpoint ids.
        let head = r#"diagram top-down

api "API" : cylinder
db "Postgres" : cylinder
cache "Redis" : cylinder

group left-right "Services"
    worker "Worker"
    queue "Queue"
    relay "Relay"
end

"#;
        let edges_a = [
            "api --> worker",
            "api --> queue",
            "api -- \"poll\" --> relay",
            "worker --> db",
            "queue --> db",
            "relay -- dotted \"pub\" --> cache",
            "cache --> api",
        ];
        let mut edges_b = edges_a.to_vec();
        edges_b.reverse();
        let build = |edges: &[&str]| {
            let mut src = head.to_string();
            for line in edges {
                src.push_str(line);
                src.push('\n');
            }
            let raw = parser::parse_diagram(&src).expect("parse");
            resolve::resolve(&raw).expect("resolve")
        };
        let la = layout_phase1(&build(&edges_a));
        let lb = layout_phase1(&build(&edges_b));

        // Node placement is edge-order independent too.
        for n in &la.nodes {
            let m = lb
                .nodes
                .iter()
                .find(|o| o.id == n.id)
                .unwrap_or_else(|| panic!("node `{}` missing after permutation", n.id));
            assert_eq!(n, m, "node `{}` moved under edge permutation", n.id);
        }

        for e in &la.edges {
            let other = lb
                .edges
                .iter()
                .find(|o| o.from == e.from && o.to == e.to)
                .unwrap_or_else(|| panic!("edge `{}->{}` missing after permutation", e.from, e.to));
            assert_eq!(
                e.points.len(),
                other.points.len(),
                "{}->{} waypoint count changed under edge permutation",
                e.from,
                e.to
            );
            for (k, (p, q)) in e.points.iter().zip(&other.points).enumerate() {
                assert_eq!(
                    p, q,
                    "{}->{} waypoint #{k} moved under edge permutation",
                    e.from, e.to
                );
            }
        }
    }

    // ================= M13 — label placement (inspection) =================
    #[test]
    #[ignore = "debug dump; run with --nocapture --ignored to inspect"]
    fn _dump_labels_for_inspection() {
        for (name, src) in [
            ("finance", include_str!("../examples/finance.dgmr")),
            ("infra", include_str!("../examples/infra.dgmr")),
            (
                "subdirection",
                include_str!("../examples/subdirection.dgmr"),
            ),
        ] {
            let (d, l) = lay_final(src);
            println!("== {name} ==");
            for (i, e) in d.edges.iter().enumerate() {
                let Some(label) = e.label.as_deref() else {
                    continue;
                };
                let Some((ax, ay)) = l.edges[i].label_at else {
                    println!("  {label:?} NO ANCHOR");
                    continue;
                };
                let (w, h) = label_box(label);
                let r = label_rect((ax, ay), (w, h));
                // peer coverage
                let hit = inflate(r, LABEL_HIT_PAD);
                let mut covered = 0.0;
                for (j, p) in l.edges.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    for s in p.points.windows(2) {
                        let c = segment_covered_len(s[0], s[1], hit);
                        if c > 0.05 {
                            println!("    covers {}->{} {:.1}px", p.from, p.to, c);
                        }
                        covered += c;
                    }
                }
                println!(
                    "  {label:?} at ({:.1},{:.1}) rect [{:.1},{:.1},{:.1},{:.1}] peer_covered={:.1}",
                    ax, ay, r.0, r.1, r.2, r.3, covered
                );
            }
        }
    }
    #[test]
    #[ignore = "trace phase-2 negotiation (DIAG_SRC = a .dgmr path)"]
    fn _trace_m18_stress() {
        let src = match std::env::var("DIAG_SRC") {
            Ok(p) => std::fs::read_to_string(p).unwrap(),
            Err(_) => "diagram top-down\na\nb\nc\n".to_string(),
        };
        let raw = parser::parse_diagram(&src).expect("parse");
        let d = resolve::resolve(&raw).expect("resolve");
        let _l = layout_trace(&d, &|msg| println!("[trace] {msg}"));
    }

    #[test]
    #[ignore = "debug dump; probe M18 (set DIAG_SRC to a .dgmr path)"]
    fn _probe_m18() {
        let src = match std::env::var("DIAG_SRC") {
            Ok(p) => std::fs::read_to_string(p).unwrap(),
            Err(_) => "diagram top-down\na\nb\nc\n".to_string(),
        };
        let (_d, l) = lay(&src, false);
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap()
                .then(a.x.partial_cmp(&b.x).unwrap())
        });
        for n in &nodes {
            println!(
                "  node {:<10} x={:7.1} y={:7.1} w={:5.1} h={:5.1}",
                n.id, n.x, n.y, n.w, n.h
            );
        }
        // M18 regression probe: the two-deep side-by-side fixture.
        let l2 = lay_final(
            r#"diagram left-right
group
group
A
B
end
end
C
A --> C
B --> C
"#,
        )
        .1;
        for e in &l2.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  [m18] {} -> {} : {}", e.from, e.to, pts.join(" "));
        }
        // M18 regression probe 2: the dense-stress overlap (n18->n19 riding
        // n16->n22's source descent at the same x).
        if let (Some(e1), Some(e2)) = (
            l.edges.iter().find(|e| e.from == "n18" && e.to == "n19"),
            l.edges.iter().find(|e| e.from == "n16" && e.to == "n22"),
        ) {
            for (nm, e) in [("n18->n19", e1), ("n16->n22", e2)] {
                let pts: Vec<String> = e
                    .points
                    .iter()
                    .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                    .collect();
                println!("  [m18] {} : {}", nm, pts.join(" "));
            }
        }
        let b2 = node_rect(&l2, "B");
        for e in &l2.edges {
            if e.from == "A" {
                for w in e.points.windows(2) {
                    if segment_intersects_rect(w[0], w[1], (b2.x, b2.y, b2.w, b2.h)) {
                        println!("  [m18] A->C SEGMENT PIERCES B: {:?}->{:?}", w[0], w[1]);
                    }
                }
            }
        }
        for s in &l.groups {
            println!(
                "  group #{} x={:.1} y={:.1} w={:.1} h={:.1}",
                s.index, s.x, s.y, s.w, s.h
            );
        }
        for e in &l.edges {
            let pts: Vec<String> = e
                .points
                .iter()
                .map(|(x, y)| format!("({:.1},{:.1})", x, y))
                .collect();
            println!("  {:<10} -> {:<10} : {}", e.from, e.to, pts.join(" "));
        }
    }

    #[test]
    fn classifier_root_level_edge_all_frames_solid() {
        // Two top-level nodes, one group they don't belong to: the group
        // frame is solid for the edge; the ancestor set is empty.
        let d = diagram_of("diagram top-down\na \"A\"\nb \"B\"\ngroup \"G\"\nx \"X\"\nend\n");
        let m = memberships(&d, &["a", "b", "x"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        assert!(
            fc.ancestors.is_empty(),
            "no ancestors for top-level endpoints"
        );
        assert_eq!(fc.solid, vec![0], "the only group is solid");
        assert_eq!(fc.of_frame(0), FrameClassKind::Solid);
        // Two top-level (`None`) endpoints share the level itself — the
        // disjointness flag is about *group chains*, and neither endpoint
        // has one.
        assert!(!fc.chains_disjoint, "no group chains to be disjoint");
    }

    #[test]
    fn classifier_both_endpoints_in_one_group() {
        // Both endpoints in the same group: that frame is the only
        // ancestor; nothing is solid.
        let d = diagram_of("diagram top-down\ngroup \"G\"\na \"A\"\nb \"B\"\nend\n");
        let m = memberships(&d, &["a", "b"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        assert_eq!(fc.ancestors, vec![0]);
        assert!(fc.solid.is_empty());
        assert_eq!(fc.of_frame(0), FrameClassKind::Ancestor);
        assert!(!fc.chains_disjoint, "same-group endpoints are not disjoint");
    }

    #[test]
    fn classifier_nested_edge_lists_the_whole_chain() {
        // `a` inside Outer>Inner, `b` inside Outer: Inner and Outer are
        // both ancestors; nothing else exists to be solid.
        let d =
            diagram_of("diagram top-down\ngroup \"O\"\ngroup \"I\"\na \"A\"\nend\nb \"B\"\nend\n");
        let m = memberships(&d, &["a", "b"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        // Outer=0, Inner=1 in declaration order.
        assert_eq!(fc.ancestors, vec![0, 1]);
        assert!(fc.solid.is_empty());
        assert!(!fc.chains_disjoint, "nested endpoint shares a chain tail");
    }

    #[test]
    fn classifier_sibling_groups_are_mutually_solid() {
        // `a --> b` with `a` in G1 and `b` in G2 (siblings): each group is
        // an ancestor for its own endpoint and solid for the other's —
        // both appear in the ancestor list, and nothing is left solid.
        let d = diagram_of(
            "diagram top-down\ngroup \"G1\"\na \"A\"\nend\ngroup \"G2\"\nb \"B\"\nend\n",
        );
        let m = memberships(&d, &["a", "b"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        assert_eq!(fc.ancestors, vec![0, 1]);
        assert!(fc.solid.is_empty());
        assert!(fc.chains_disjoint);
    }

    #[test]
    fn classifier_unrelated_subtree_is_solid() {
        // `a` in G1, `b` top-level, and G1 contains a nested subgroup H:
        // H is solid for the edge (it is on neither endpoint's chain);
        // G1 is an ancestor.
        let d = diagram_of(
            "diagram top-down\ngroup \"G1\"\ngroup \"H\"\nx \"X\"\nend\na \"A\"\nend\nb \"B\"\n",
        );
        let m = memberships(&d, &["a", "b", "x"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        assert_eq!(fc.ancestors, vec![0]);
        assert_eq!(fc.solid, vec![1], "the unrelated nested frame is solid");
        assert_eq!(fc.of_frame(1), FrameClassKind::Solid);
    }

    #[test]
    fn classifier_deep_nesting_wall_of_siblings() {
        // Deep nesting: `a` in Root > L1 > L2 > L3, `b` top-level, plus a
        // sibling wall (L2's other child L2s with its own subtree). Every
        // Root/L1/L2/L3 frame is an ancestor; the sibling subtree's frames
        // are all solid.
        let d = diagram_of(
            "diagram top-down\ngroup \"R\"\ngroup \"L1\"\ngroup \"L2\"\ngroup \"L3\"\na \"A\"\nend\ngroup \"L2s\"\nl2sx \"S\"\nend\nend\nend\nend\nb \"B\"\n",
        );
        let m = memberships(&d, &["a", "b", "l2sx"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        // Root=0, L1=1, L2=2, L3=3, L2s=4.
        assert_eq!(fc.ancestors, vec![0, 1, 2, 3]);
        assert_eq!(fc.solid, vec![4]);
        // `b` is top-level (`None`) — a top-level endpoint's chain is the
        // empty group chain, so no group-vs-group containment applies;
        // the sibling subtree is still solid for the edge.
        assert!(!fc.chains_disjoint);
    }

    #[test]
    fn classifier_nesting_endpoint_pair() {
        // An edge from a node in Root to a node in Root > Inner: Root is
        // the shared chain tail (not disjoint); Inner is an ancestor; the
        // sibling S of Inner inside Root is solid.
        let d = diagram_of(
            "diagram top-down\ngroup \"R\"\ngroup \"I\"\na \"A\"\nend\ngroup \"S\"\nx \"X\"\nend\nb \"B\"\nend\n",
        );
        let m = memberships(&d, &["a", "b"]);
        let fc = FrameClass::of(m[0], m[1], &d.groups);
        // Root=0, Inner=1, S=2.
        assert_eq!(fc.ancestors, vec![0, 1]);
        assert_eq!(fc.solid, vec![2]);
        assert!(!fc.chains_disjoint, "b's group contains a's group");
    }

    #[test]
    fn frame_world_bands_and_solids_match_the_classification() {
        // Build a Geometry with hand-placed frames: the world's band field
        // covers the ancestors' four bands each; the solid field carries
        // the unrelated frame's raw rect.
        let d = diagram_of(
            "diagram top-down\ngroup \"G1\"\na \"A\"\nend\ngroup \"G2\"\nb \"B\"\nend\ngroup \"G3\"\nx \"X\"\nend\n",
        );
        let mut id_index = std::collections::HashMap::new();
        id_index.insert("a", 0usize);
        id_index.insert("b", 1usize);
        id_index.insert("x", 2usize);
        let mut frame_rect = std::collections::HashMap::new();
        frame_rect.insert(0usize, (0.0, 0.0, 100.0, 100.0));
        frame_rect.insert(1usize, (200.0, 0.0, 100.0, 100.0));
        frame_rect.insert(2usize, (100.0, 200.0, 100.0, 100.0));
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect: std::collections::HashMap::new(),
            frame_rect,
            direct_pts: std::collections::HashMap::new(),
            level_items: std::collections::HashMap::new(),
        };
        // a in G1 (0), b in G2 (1), G3 (2) unrelated.
        let fw = EdgeFrameWorld::build(&geom, Some(0), Some(1));
        assert_eq!(fw.class.ancestors, vec![0, 1]);
        assert_eq!(fw.class.solid, vec![2]);
        assert_eq!(fw.ancestor_bands.len(), 8, "4 bands per ancestor frame");
        assert_eq!(fw.solid, vec![(100.0, 200.0, 100.0, 100.0)]);
        // A route straight between the frames (a horizontal leg through
        // the open space between them) is clear of the frame world.
        assert!(fw.seg_clear((100.0, 50.0), (200.0, 50.0)));
        // A route riding the top band of G1 parallel is barred.
        assert!(!fw.seg_clear((10.0, 0.0), (90.0, 0.0)));
        // A route through the solid frame's interior is barred.
        assert!(!fw.seg_clear((150.0, 150.0), (150.0, 250.0)));
        assert!(fw.cross_cost(&[(100.0, 50.0), (200.0, 50.0)]) < 1e-6);
    }

    #[test]
    fn around_region_widens_beyond_obstacles_and_clamps_to_canvas() {
        // A frame world with one ancestor frame; nodes well inside the
        // canvas. The region must extend AROUND_MARGIN past every
        // obstacle extent and stay inside the canvas.
        let d = diagram_of("diagram top-down\ngroup \"G\"\na \"A\"\nend\n");
        let mut id_index = std::collections::HashMap::new();
        id_index.insert("a", 0usize);
        let mut frame_rect = std::collections::HashMap::new();
        frame_rect.insert(0usize, (100.0, 100.0, 200.0, 100.0));
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect: std::collections::HashMap::new(),
            frame_rect,
            direct_pts: std::collections::HashMap::new(),
            level_items: std::collections::HashMap::new(),
        };
        let fw = EdgeFrameWorld::build(&geom, Some(0), None);
        let nodes = vec![(120.0, 120.0, 40.0, 40.0)];
        let region = around_region(&fw, &nodes, (1000.0, 800.0));
        // The world's extent is the *band* field: a band sits half the
        // stroke (BORDER_HALF, 0.75) outside the frame rect and the
        // region adds AROUND_MARGIN beyond that.
        let e = 100.0 - BORDER_HALF - AROUND_MARGIN;
        assert!((region.0 - e).abs() < 1e-2);
        assert!((region.1 - e).abs() < 1e-2);
        assert!((region.0 + region.2 - (300.0 + BORDER_HALF + AROUND_MARGIN)).abs() < 1e-2);
        assert!((region.1 + region.3 - (200.0 + BORDER_HALF + AROUND_MARGIN)).abs() < 1e-2);
        // Clamped to the canvas when the margin would pass it.
        let tight = around_region(&fw, &nodes, (180.0, 150.0));
        assert!(tight.0 >= 0.0 && tight.1 >= 0.0);
        assert!(tight.0 + tight.2 <= 180.0 + 1e-2);
        assert!(tight.1 + tight.3 <= 150.0 + 1e-2);
    }

    #[test]
    fn track_route_in_region_finds_around_the_wall_detour_the_local_box_hides() {
        // The region-widening payoff (still off the live pipeline): a
        // solid frame wall sits between the two ports, extending far
        // sideways. The local region (ports + obstacles' own extent)
        // offers no way around; the widened region reaches past the
        // wall's edge and finds the detour.
        let d = diagram_of("diagram top-down\nw \"W\"\na \"A\"\nb \"B\"\n");
        let mut id_index = std::collections::HashMap::new();
        id_index.insert("a", 0usize);
        id_index.insert("b", 1usize);
        id_index.insert("w", 2usize);
        let mut node_rect = std::collections::HashMap::new();
        node_rect.insert(0usize, (0.0, 0.0, 40.0, 40.0)); // a
        node_rect.insert(1, (400.0, 0.0, 40.0, 40.0)); // b
        node_rect.insert(2, (200.0, 0.0, 40.0, 300.0)); // w — the wall itself
        // The wall between them, flush to the canvas top: no room above.
        let wall = (200.0, 0.0, 40.0, 300.0);
        let mut frame_rect = std::collections::HashMap::new();
        frame_rect.insert(0usize, wall);
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect: node_rect.clone(),
            frame_rect,
            direct_pts: std::collections::HashMap::new(),
            level_items: std::collections::HashMap::new(),
        };
        let mut obs = Obstacles::empty();
        obs.push_node(inflate(wall, BORDER_HALF));
        // Ports on facing sides at the wall's y-band. The wall is pushed
        // as a *solid* rect (the fully-blocked semantics an unrelated
        // frame gets from the classifier) and is flush to the canvas
        // top: no corridor exists above it at all, so the only way
        // across is around its bottom.
        let a = (40.0, 40.0);
        let b = (400.0, 40.0);
        // Local region (track_route's own derived one): its top boundary
        // is the wall top − TRACK_OFF — *above* the canvas top — so the
        // route it finds runs over the wall's top edge, outside the
        // drawn world. That is exactly the defect the region widening
        // exists to close: the local region cannot see the canvas
        // boundary, so it "detours" through undrawable space. The
        // widened region is clamped to the canvas, so the same search
        // must take the real corridor below the wall instead.
        let local = track_route(a, b, &obs, Escapes::default());
        let local_min_y = local.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        assert!(
            !local.is_empty() && local_min_y < 0.0,
            "the local route must escape above the canvas (the defect)"
        );
        // Widened region: the around-margin reaches past the wall bottom
        // *within* the canvas. The frame world is empty here (a
        // hand-built obstacle world, not a classified one — `w` is a
        // top-level node, no groups exist), so the region is built from
        // the caller's node list directly; assert its shape and use it.
        let fw = EdgeFrameWorld::build(&geom, None, None);
        assert!(fw.solid.is_empty() && fw.ancestor_bands.is_empty());
        // The world extent = the wall (the only node passed) + margin —
        // but the *node rects* the caller passes are the edge's own
        // endpoint nodes too, which span the ports; include them so the
        // region actually contains both ports (a region that lost a port
        // routes nothing, the bounded convention).
        let region = around_region(
            &fw,
            &node_rect.values().copied().collect::<Vec<_>>(),
            (500.0, 400.0),
        );
        // Left/top clamped at the canvas (the `a` node sits at the
        // origin); right: past `b` (440) + margin → the canvas edge
        // (500); bottom: the wall bottom (300) + margin → clamped to
        // 360? No — 300 + 60 = 360, inside the canvas: exactly 360.
        assert!((region.0 - 0.0).abs() < 1e-2, "clamped at the canvas left");
        assert!((region.1 - 0.0).abs() < 1e-2, "clamped at the canvas top");
        assert!((region.0 + region.2 - 500.0).abs() < 1e-2);
        assert!((region.1 + region.3 - 360.0).abs() < 1e-2);
        let wide = track_route_in_region(a, b, &obs, Escapes::default(), region);
        assert!(!wide.is_empty(), "widened region must find the detour");
        assert!(obs.clear(&wide), "the detour must clear the wall");
        // It must actually go around the wall (drop below the wall
        // bottom — the corridor's track row is the wall bottom +
        // TRACK_OFF), and stay inside the canvas.
        let max_y = wide.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
        assert!(max_y > wall.1 + wall.3 - 1.0, "detour must pass the wall");
        assert!(
            wide.iter().all(|&(_, y)| y >= 0.0),
            "the detour must stay within the canvas"
        );
    }

    #[test]
    fn whole_path_route_charges_frame_crossings() {
        // The cross-cost hook: a world with one ancestor band; a route
        // that crosses it twice (down then back) costs more than a
        // straight crossing. Whole-path_route itself is exercised
        // end-to-end in M20.2; here the cost term plus the widened-region
        // route run together.
        let d = diagram_of("diagram top-down\ngroup \"G\"\na \"A\"\nend\n");
        let mut id_index = std::collections::HashMap::new();
        id_index.insert("a", 0usize);
        let mut frame_rect = std::collections::HashMap::new();
        frame_rect.insert(0usize, (100.0, 100.0, 200.0, 100.0));
        let geom = Geometry {
            diagram: &d,
            id_index,
            node_rect: std::collections::HashMap::new(),
            frame_rect,
            direct_pts: std::collections::HashMap::new(),
            level_items: std::collections::HashMap::new(),
        };
        let fw = EdgeFrameWorld::build(&geom, Some(0), None);
        // One straight perpendicular crossing.
        let one = vec![(150.0, 50.0), (150.0, 250.0)];
        // Out-and-back: two crossings of the same band.
        let two = vec![(150.0, 50.0), (150.0, 250.0), (250.0, 250.0), (250.0, 50.0)];
        assert!(fw.cross_cost(&one) < fw.cross_cost(&two) - FRAME_CROSS_PENALTY);
        // And a designated-crossing end (endpoint on the centerline) is
        // free: `one` ends inside the frame, so score the same shape
        // split at the centerline — the cost counts only mid-segment
        // transversals.
        let touching = vec![(150.0, 50.0), (150.0, 150.0), (150.0, 250.0)];
        assert!(fw.cross_cost(&touching) < fw.cross_cost(&two) - FRAME_CROSS_PENALTY);
    }

    // ---- M20.3: the search is the cross-boundary router ----

    /// The whole-path search is the router for a cross-boundary edge: the
    /// route is a single polyline from the source node's port to the target
    /// node's port, and it crosses the group frame where the search chose —
    /// no rep port, no piecewise stub seam. (Before M20.3 the route was
    /// assembled from a within-frame stub, an LCA segment, and a second stub,
    /// and the frame crossings were rep ports at fixed coordinates.)
    #[test]
    fn cross_boundary_edges_route_by_the_whole_path_search() {
        let (_d, l) = lay_final(
            "diagram top-down\n\
             src\n\
             group \"S\"\n\
             t\n\
             end\n\
             src --> t\n",
        );
        let e = edge_path(&l, "src", "t");
        let src = node_rect(&l, "src");
        let t = node_rect(&l, "t");
        // The polyline starts on the source's boundary and ends on the
        // target's — the two node ports, nothing in between standing in for
        // a frame rep.
        assert!(on_boundary((src.x, src.y, src.w, src.h), e.points[0]));
        assert!(on_boundary((t.x, t.y, t.w, t.h), *e.points.last().unwrap()));
        assert!(is_orthogonal(&e.points));
        // It crosses the frame's top border once, on the way in.
        let s = sub_rect(&l, 0);
        let crossings = e
            .points
            .iter()
            .filter(|&&(x, y)| (y - s.y).abs() < 1e-2 && x > s.x && x < s.x + s.w)
            .count();
        assert!(
            crossings >= 1,
            "no crossing of the frame top: {:?}",
            e.points
        );
    }

    /// Titles are solid obstacles for the search (M20.3): no edge segment
    /// crosses a group's drawn title text on any sample. The deleted title
    /// detour hand-built exactly this clearance; the search now discovers it.
    #[test]
    fn every_edge_lands_on_its_endpoint_boundary_on_the_samples() {
        for src in [
            include_str!("../examples/infra.dgmr"),
            include_str!("../examples/finance.dgmr"),
            include_str!("../examples/subdirection.dgmr"),
            include_str!("../examples/tencent.dgmr"),
            include_str!("../examples/cluster.dgmr"),
            include_str!("../examples/sides.dgmr"),
        ] {
            let (_d, l) = lay_final(src);
            for e in &l.edges {
                let a = node_rect(&l, &e.from);
                let b = node_rect(&l, &e.to);
                let first = e.points[0];
                let last = *e.points.last().unwrap();
                assert!(
                    on_boundary((a.x, a.y, a.w, a.h), first),
                    "{}->{} starts off {}'s boundary: {first:?} vs {a:?}",
                    e.from,
                    e.to,
                    e.from
                );
                assert!(
                    on_boundary((b.x, b.y, b.w, b.h), last),
                    "{}->{} ends off {}'s boundary: {last:?} vs {b:?}",
                    e.from,
                    e.to,
                    e.to
                );
            }
        }
    }

    #[test]
    fn no_edge_crosses_a_group_title() {
        for src in [
            include_str!("../examples/infra.dgmr"),
            include_str!("../examples/finance.dgmr"),
            include_str!("../examples/subdirection.dgmr"),
            include_str!("../examples/tencent.dgmr"),
            include_str!("../examples/cluster.dgmr"),
        ] {
            let (d, l) = lay_final(src);
            for s in &l.groups {
                let Some(title) = d.groups[s.index].title.as_deref() else {
                    continue;
                };
                let tr = title_text_rect((s.x, s.y, s.w, s.h), title);
                for e in &l.edges {
                    for w in e.points.windows(2) {
                        assert!(
                            !segment_intersects_rect(w[0], w[1], tr),
                            "{}->{} crosses the title {title:?} of frame #{}: {:?}->{:?}",
                            e.from,
                            e.to,
                            s.index,
                            w[0],
                            w[1]
                        );
                    }
                }
            }
        }
    }

    /// A forced side on a cross-boundary edge still binds the node port
    /// (M11 semantics): the port lands on the requested side and the route
    /// leaves it outward. The search owns the span between the ports, so the
    /// deleted rep model's coupled/decoupled duality no longer applies — the
    /// frame crossing is just wherever the route goes.
    #[test]
    fn forced_cross_boundary_ports_are_honored_by_the_search() {
        for side in ["top", "bottom", "left", "right"] {
            let src = format!(
                "diagram top-down\nsrc\ngroup \"S\"\nt \"Target\"\nend\nsrc -- to=\"{side}\" --> t\n"
            );
            let (_d, l) = lay(&src, false);
            let t = node_rect(&l, "t");
            let e = edge_path(&l, "src", "t");
            assert!(
                on_side(t, *e.points.last().unwrap(), side),
                "to=\"{side}\": end {:?} not on t's {side} (t = {t:?})",
                e.points.last().unwrap()
            );
            assert!(is_orthogonal(&e.points), "to=\"{side}\": {:?}", e.points);
        }
    }

    /// The search leaves the graph's unrelated (solid) frames alone: an edge
    /// between two top-level nodes never punctures a sibling group's frame,
    /// even when the straight line would. (The M20 obstacle model's
    /// by-construction rule — this is the property the deleted rep model's
    /// cost tuning only approximated.)
    #[test]
    fn whole_path_search_never_punctures_an_unrelated_frame() {
        // `a --> b` spans a whole sibling group that sits between them (the
        // group is ranked between `a` and `b` by its own two edges). The
        // group is *unrelated* to `a --> b` — neither endpoint is a member —
        // so the M20 obstacle model makes it solid and the route must go
        // around it rather than through it (the by-construction rule the
        // deleted rep model's cost tuning only approximated).
        let (_d, l) = lay_final(
            "diagram top-down\n\
             a \"A\"\n\
             group \"Wall\"\n\
             w1 \"W1\"\n\
             w2 \"W2\"\n\
             w1 --> w2\n\
             end\n\
             b \"B\"\n\
             a --> w1\n\
             w2 --> b\n\
             a --> b\n",
        );
        let wall = sub_rect(&l, 0);
        let wall_rect = (wall.x, wall.y, wall.w, wall.h);
        let e = edge_path(&l, "a", "b");
        // The fixture is meaningful only if the straight line really would
        // cut the frame — otherwise the assertion below is vacuous.
        let a = node_rect(&l, "a");
        let b = node_rect(&l, "b");
        assert!(
            segment_intersects_rect(
                (a.x + a.w / 2.0, a.y + a.h),
                (b.x + b.w / 2.0, b.y),
                wall_rect
            ),
            "fixture is vacuous: the straight a->b line misses the wall"
        );
        for w in e.points.windows(2) {
            assert!(
                !segment_intersects_rect(w[0], w[1], wall_rect),
                "a->b segment {:?}->{:?} punctures the unrelated frame {wall_rect:?}",
                w[0],
                w[1]
            );
        }
    }

    /// Determinism survives the new regime: two layouts of the same source
    /// produce byte-identical routes (the whole-path search has no
    /// iteration-order dependence).
    #[test]
    fn cross_boundary_routes_are_deterministic() {
        let src = include_str!("../examples/finance.dgmr");
        let (_d1, l1) = lay_final(src);
        let (_d2, l2) = lay_final(src);
        for (a, b) in l1.edges.iter().zip(&l2.edges) {
            assert_eq!(
                a.points, b.points,
                "{}->{} moved between runs",
                a.from, a.to
            );
        }
    }
}
