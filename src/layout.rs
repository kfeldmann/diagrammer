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
//!   escape). The LCA-level segment connects the two representatives' *ports*
//!   (at the frame centers, on the sides facing each other along the LCA's
//!   direction axis — the same port the flat engine computes for any edge at
//!   that level). Each endpoint node then *fans* from its own port (same side,
//!   at the node's center) to its representative's frame port; because the
//!   frame port sits at the frame's center, the fan stays on the endpoint's
//!   own side of the frame rather than cutting across its siblings. Stub
//!   segments that traverse intermediate (nested) frames are clipped at each
//!   frame's boundary so the path records where it crosses. M7 will make these
//!   segments orthogonal; for M5 they may be diagonal.

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

/// A subgraph's frame rectangle, in absolute (page) coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct SubgraphRect {
    /// Index into `Diagram::subgraphs`.
    pub index: usize,
    pub title: Option<String>,
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

/// Lay out a resolved [`Diagram`], honoring subgraph containment and
/// per-subgraph direction.
pub fn layout(diagram: &Diagram) -> Layout {
    if diagram.nodes.is_empty() {
        // Still produce frames for any (empty) subgraphs, sized to their
        // padding, so they render as small labeled boxes rather than vanish.
        let top = layout_level(diagram, None, diagram.direction, &[]);
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

    let top = layout_level(diagram, None, diagram.direction, &edge_infos);
    assemble(diagram, top, &edge_infos)
}

/// Turn a top-level [`LevelOut`] (page coordinates) into the public
/// [`Layout`], routing cross-boundary edges through frame connection points
/// (M5). Direct internal edges keep the flat engine's waypoints; every other
/// edge is rebuilt as a frame-aware path in [`cross_boundary_path`].
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
        let points = if let Some(pts) = direct_pts.get(&ei) {
            pts.clone()
        } else {
            // Cross-boundary edge (M5): route through connection points on the
            // subgraph frames it crosses, without disturbing the groups'
            // internal layout. The LCA-level segment connects the two
            // representatives' ports (at the frame centers, along the LCA's
            // direction axis); within-frame stubs fan from each endpoint node
            // to its representative's port, clipping at any intermediate
            // (nested) frame boundaries.
            let fi = id_index[e.from.as_str()];
            let ti = id_index[e.to.as_str()];
            let from = node_rect.get(&fi).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            let to = node_rect.get(&ti).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            let lca = edge_infos.get(ei).map(|i| i.lca).unwrap_or(None);
            let lca_dir = effective_direction(lca, diagram);
            let from_chain =
                chain_to_lca(diagram.nodes[fi].group, lca, &diagram.subgraphs);
            let to_chain =
                chain_to_lca(diagram.nodes[ti].group, lca, &diagram.subgraphs);
            cross_boundary_path(from, to, &from_chain, &to_chain, lca_dir, &|s| {
                frame_rect.get(&s).copied().unwrap_or((0.0, 0.0, 0.0, 0.0))
            })
        };
        edges_out.push(EdgePath {
            from: e.from.clone(),
            to: e.to.clone(),
            points,
        });
    }

    let mut subgraphs_out = Vec::with_capacity(diagram.subgraphs.len());
    for (gi, sg) in diagram.subgraphs.iter().enumerate() {
        let (x, y, w, h) = top
            .frames
            .iter()
            .find(|(idx, _, _, _, _)| *idx == gi)
            .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
            .unwrap_or((0.0, 0.0, 0.0, 0.0));
        subgraphs_out.push(SubgraphRect {
            index: gi,
            title: sg.title.clone(),
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
        let out = layout_level(diagram, Some(cs), child_dir, edge_infos);
        let top_inset = if diagram.subgraphs[cs].title.is_some() {
            FRAME_TITLE_H
        } else {
            FRAME_PAD_Y
        };
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
            let h = (m.height + 2.0 * NODE_PAD_Y).max(MIN_NODE_H);
            item_of.insert(ItemRef::Node(gi), items.len());
            item_refs.push(ItemRef::Node(gi));
            items.push(FlatItem { w, h });
        }
    }
    for &cs in &child_subs {
        let child = child_outs.get(&cs).expect("child out just inserted");
        let top_inset = if diagram.subgraphs[cs].title.is_some() {
            FRAME_TITLE_H
        } else {
            FRAME_PAD_Y
        };
        let frame_w = child.out.w + 2.0 * FRAME_PAD_X;
        let frame_h = child.out.h + top_inset + FRAME_PAD_Y;
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

/// Frame-aware path for a cross-boundary edge from `from` to `to` (absolute
/// node rects).
///
/// `from_chain` / `to_chain` list the subgraph frames containing each endpoint,
/// innermost first, ending with the representative at the LCA level (the
/// outermost frame the edge crosses on that side). An empty chain means the
/// endpoint is a direct child of the LCA, so the endpoint node *is* its own
/// representative. `frame_rect(i)` returns the absolute frame rect.
///
/// The path runs:
/// `from`-port → [intermediate source frame crossings] → source-rep port →
/// target-rep port → [intermediate target frame crossings] → `to`-port.
/// The middle (rep-port → rep-port) is the LCA-level segment, a straight line
/// between the representatives' ports at their frame centers along the LCA's
/// direction axis. Each within-frame stub *fans* from the endpoint node's
/// own port (same side, at the node's center) to its representative's port;
/// because the rep port is at the frame's center, the fan stays on the
/// endpoint's side of the frame and does not cut across the frame's other
/// members. Intermediate nested frames are clipped at their boundaries.
fn cross_boundary_path(
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    from_chain: &[usize],
    to_chain: &[usize],
    lca_dir: Direction,
    frame_rect: &dyn Fn(usize) -> (f32, f32, f32, f32),
) -> Vec<(f32, f32)> {
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
    let rep_from_port = port(rep_from, exit);
    let rep_to_port = port(rep_to, entry);
    let from_port = port(from, exit);
    let to_port = port(to, entry);

    let mut pts = Vec::new();

    // Source side: fan from the endpoint node to its representative's port,
    // clipping at any intermediate (nested) source frames along the way.
    if !from_chain.is_empty() {
        pts.push(from_port);
        for &s in &from_chain[..from_chain.len() - 1] {
            if let Some(p) = line_rect_exit(from_port, rep_from_port, frame_rect(s)) {
                pts.push(p);
            }
        }
    }
    // LCA-level segment start (also the source port when `from` is its own
    // representative, i.e. `from_chain` is empty).
    pts.push(rep_from_port);
    // LCA-level segment end.
    pts.push(rep_to_port);
    // Target side: fan from the representative's port down to the endpoint
    // node, clipping at intermediate target frames (entered outermost-first).
    if !to_chain.is_empty() {
        for &s in to_chain[..to_chain.len() - 1].iter().rev() {
            if let Some(p) = line_rect_exit(rep_to_port, to_port, frame_rect(s)) {
                pts.push(p);
            }
        }
        pts.push(to_port);
    }

    dedup_consecutive(&mut pts);
    pts
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
    for (_r, layer) in layers.iter().enumerate() {
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
    order: &mut Vec<usize>,
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
    for (_r, layer) in layers.iter().enumerate() {
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
    layers: &mut Vec<Vec<usize>>,
    order: &mut Vec<usize>,
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

    fn sub_rect<'a>(l: &'a Layout, idx: usize) -> &'a SubgraphRect {
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
    fn _dump_infra_for_inspection() {
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        println!("canvas: {:.1} x {:.1}", l.width, l.height);
        let mut nodes = l.nodes.clone();
        nodes.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap().then(a.x.partial_cmp(&b.x).unwrap()));
        for n in &nodes {
            println!("  {:<14} x={:7.1} y={:7.1} w={:5.1} h={:5.1}", n.id, n.x, n.y, n.w, n.h);
        }
        for s in &l.subgraphs {
            println!("  subgraph #{} {:?} x={:.1} y={:.1} w={:.1} h={:.1}", s.index, s.title, s.x, s.y, s.w, s.h);
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
    fn cylinder_shapes_are_distinct_but_measured_same_way() {
        // Shapes don't change v0 box geometry; just ensure layout still works.
        let (_d, l) = lay("diagram top-down\ndb \"Pg\" : cylinder\ncache \"Redis\" : cylinder\ndb-->cache\n");
        assert_all_finite(&l);
        assert_no_overlaps(&l);
        // `Shape::Cylinder` is parsed; the rect is still a box for v0.
        let _ = Shape::Cylinder;
    }

    // ================= Compound layout (M4) =================

    #[test]
    fn subgraph_frame_contains_its_members() {
        let (_d, l) = lay(
            "diagram top-down\n\
             subgraph \"Cluster\"\n\
             a\n\
             b\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 1);
        assert_eq!(l.nodes.len(), 2);
        let f = sub_rect(&l, 0);
        assert_eq!(f.title.as_deref(), Some("Cluster"));
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
        let (_d, l) = lay(
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
        assert_eq!(outer.title.as_deref(), Some("Outer"));
        assert_eq!(inner.title.as_deref(), Some("Inner"));
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
        let (_d, l) = lay(
            "diagram top-down\n\
             subgraph \"Empty\"\n\
             end\n",
        );
        assert_eq!(l.subgraphs.len(), 1);
        let f = sub_rect(&l, 0);
        assert_eq!(f.title.as_deref(), Some("Empty"));
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
        // The reason the within-frame stub *fans* to the frame's center port
        // (rather than running perpendicular to the inner node): with two
        // members in a frame and an edge from the right member to a node
        // below-left, the stub must not cut across the left member. This is
        // the Mermaid/dagre failure mode this tool exists to escape.
        let (_d, l) = lay(include_str!("../examples/infra.mmd"));
        let api1 = node_rect(&l, "api1");
        let k8s = sub_rect(&l, 0);
        let e = edge_path(&l, "api2", "db");
        // The stub is the first segment: api2's port -> the K8s frame port.
        assert!(e.points.len() >= 3, "api2->db should route through K8s's frame");
        let stub = (e.points[0], e.points[1]);
        assert!(
            !segment_intersects_rect(stub.0, stub.1, rect_of(api1)),
            "api2->db stub crosses api1: stub={:?} api1={:?}",
            stub,
            api1
        );
        // And the connection point on K8s sits at its center (the rep port).
        assert!((e.points[1].0 - (k8s.x + k8s.w / 2.0)).abs() < 1e-2);
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
}
