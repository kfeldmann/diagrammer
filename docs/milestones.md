# Milestones

Tracking doc for diagrammer. The grammar is specified in
[`grammar.md`](grammar.md); this file records what's done, what's next, and
how the remaining work depends on itself.

**Status legend:** ✅ done · ▶ next · ⬜ planned

**Current position:** M0, M1, M2, M3, M4, M5, M5.5, M6, M7, and M7.5 are complete. **M8 (CLI polish and extras) is next.**

---

## ✅ M0 — Grammar spec

Define the v0 input language: readable, line-oriented, Mermaid-inspired but
not backward-compatible. See [`grammar.md`](grammar.md).

- [x] Header + direction
- [x] Nodes (bare id / `id "label"`, `: shape`, attrs)
- [x] Edges (`-->`, `-- style "label" attrs -->`)
- [x] Subgraphs (visual grouping; direction slot reserved)
- [x] Comments (`#`), quoted strings with escapes
- [x] Decisions locked: `diagram` header; full-word directions; `-- "label" -->`
      edge labels; implicit node declarations; `#` comments; strict unknown
      attributes; styles `solid | dotted | dashed | thick`.

## ✅ M1 — Parser + semantic validation

Parse the full v0 language into a raw AST and validate it into a resolved
diagram. Layout and rendering are stubbed (`todo!()`).

- [x] Lexer: whitespace/comments, identifiers (hyphen rule so `A-->B` works),
      quoted strings with `\"`/`\\` escapes, line endings.
- [x] Parser (winnow): diagram header, nodes, edge chains, subgraphs (nested),
      attributes. Errors carry a byte offset + context labels → 1-based line
      numbers.
- [x] Resolver: dedup nodes, label/shape/attribute consistency on
      redeclaration, strict attribute validation (unknown & duplicate → error),
      subgraph membership (a node may belong to at most one subgraph), reject
      per-subgraph direction.
- [x] Cross-boundary edges do **not** relocate a node (the Mermaid/dagre
      behavior we want to escape). Standalone occurrences and new implicit
      declarations set membership; edge endpoints are pure references.
- [x] Minimal CLI: `diagrammer <input.mmd>` validates and prints a summary
      (`ok: N nodes, M edges, K subgraphs (direction ...)`).
- [x] Test suite: 20 tests covering parse, resolve, and error cases.

---

## ✅ M2 — Layout engine (Sugiyama-style)

Produce coordinates and a ranking for a single flat graph under one global
direction. No subgraphs yet.

- [x] **Text measurement** (precise, via `ab_glyph`): node label → pixel
      width/height. Needed to size boxes before placing them.
- [x] **Cycle removal** (break back-edges with a heuristic; restore later).
- [x] **Layering** (assign each node to a rank along the chosen direction).
- [x] **Crossing minimization** (reorder nodes within layers).
- [x] **Coordinate assignment** (x/y positions, spacing, margins).
- [x] Output a `Layout` struct (node rects + edge waypoints) consumed by M3.

Implemented in `src/layout.rs` + `src/text.rs`: an iterative DFS reverses
back-edges (tracked so arrows keep their original direction); longest-path
ranking; long edges split into zero-size **dummy** nodes (one per intermediate
rank); barycenter crossing minimisation with up/down sweeps, keeping the best
ordering; centered block x-placement (each non-source rank centered on
its parents' centroid; overlap-free — refined in M5.5) and per-rank band
y-placement. Everything is computed in canonical top-down space
then rigidly transformed into the requested direction, so edge ports stay
glued to the correct side of each box in all four directions. Text is measured
with `ab_glyph` against the DejaVu Sans Regular embedded via the `dejavu`
crate (deterministic across machines). 16 layout tests + 4 measurement tests.

**Blocked by:** nothing. **Blocks:** M3, M4.

## ✅ M3 — SVG rendering + CLI output

Emit self-contained static SVG from a `Layout`. Wire the CLI to write a file.
GitHub-renderable (no scripts/external refs).

- [x] Box rendering (rounded rect + centered label).
- [x] **Diagonal/direct edges** to start (straight lines), arrowheads via
      `<marker>`.
- [x] SVG document wrapper (viewBox sized to content).
- [x] CLI: `diagrammer <input.mmd> -o <output.svg>` (and read direction/output
      from flags).
- [x] Snapshot test harness (golden `.svg` files) so output is locked early.

Implemented in `src/render/svg.rs` + `src/main.rs`: `render_svg(diagram,
layout)` draws edges first (polylines through the layout waypoints, with a
single auto-oriented `<marker>` arrowhead, `markerUnits=userSpaceOnUse` so the
arrow keeps a fixed size) and rounded-rect boxes on top (`dominant-baseline=
"central"` + `text-anchor="middle"` for centered labels; XML-escaped). The
document sets `xmlns`, a content-sized `viewBox`, and matching `width`/
`height` — no scripts, no external refs, no CSS. The CLI gained `-o`/
`--output` (order-independent) plus `--help`; without `-o` it still prints the
validation summary. A golden-file harness under `src/render/svg.rs` compares
six diagrams against committed `snapshots/*.svg` (regenerated via
`UPDATE_SNAPSHOTS=1 cargo test`). Per-edge styles/colors/labels and the
`cylinder` shape are parsed but deliberately deferred to M6.

**Blocked by:** M2. **Note:** orthogonal edges landed in M7.

## ✅ M4 — Subgraphs with per-subgraph direction (headline feature)

Extend the layout to model compound nodes (subgraphs) and honor a per-subgraph
direction. This is the core reason the tool exists.

- [x] Compound layout: subgraphs contain child nodes/subgraphs; parents size
      to fit children.
- [x] Honor `subgraph <direction>` (lift the M1 rejection into layout).
- [x] Subgraph frame rendering (labeled border around members).
- [x] Tests: same graph laid out with different per-subgraph directions.

Implemented in `src/layout.rs` + `src/render/svg.rs` (+ containment fields in
`src/ast.rs` / `src/resolve.rs`): the flat M2 engine was extracted into a
parameterized `layout_flat(items, edges, direction, margin)` and driven
**recursively** per level. Each level (the top-level diagram or a subgraph)
lays out its direct-child real nodes plus its direct-child subgraphs as opaque
compound boxes; a subgraph box is sized to fit its own recursively-laid-out
contents plus a labeled frame, and uses its own effective direction (its own
`direction`, or the direction inherited from the enclosing level). Each edge
is assigned to the lowest common ancestor level of its endpoints and drawn
between their *representatives* there, so compound boxes are positioned
relative to their connected neighbors without dragging the inner nodes'
internal arrangement around — the Mermaid/dagre failure mode this tool exists
to escape. Edges internal to a level render via the flat waypoints;
cross-boundary edges (the M5 scope) are drawn for now as direct straight
lines between the actual endpoint node ports, which may cross a frame.
Frames are lighter/thinner rounded rects with a top-left title, rendered
behind edges and nodes. 10 new layout tests + 1 render snapshot
(`snapshots/subdirection.svg`); `snapshots/infra.svg` regenerated.

**Blocked by:** M2 (core layered layout), M3 (rendering). **Blocks:** M5.

## ✅ M5 — Cross-boundary edges that respect group layout

Edges crossing subgraph boundaries must route without collapsing the group's
internal layout (the Mermaid failure mode).

- [x] Edge routing aware of compound boundaries.
- [x] Connection points on subgraph frames.
- [x] Tests: cross-boundary edges preserve per-subgraph direction.

Implemented in `src/layout.rs` (`cross_boundary_path` plus `Side` / `port` /
`sides_along` / `line_rect_exit` / `chain_to_lca` / `effective_direction`,
replacing the M4 `straight_line` interim): a cross-boundary edge is rebuilt in
`assemble` as `from`-port → [intermediate source frame crossings] →
source-rep port → target-rep port → [intermediate target frame crossings] →
`to`-port. The LCA-level segment connects the two representatives' **ports** —
at the frame centers, on the sides facing each other along the LCA's
direction axis (the same port the flat engine computes for any edge at that
level), read off from the reps' positions so it is correct for all four
directions. Each within-frame stub then *fans* from the endpoint node's own
port (same side, at the node's center) to its representative's frame port;
because the frame port sits at the frame's **center**, the fan stays on the
endpoint's own side of the frame and does not cut across the frame's other
members — the api2→db stub no longer crosses api1, which the perpendicular
alternative would have done. Intermediate (nested) frames are clipped at their
boundaries via line/rect intersection so the path records each frame it
passes through. The groups' internal layouts are untouched (still computed by
the M4 recursion), so a left-right subgraph chain stays horizontal even while
edges cross its frame. The LCA direction is reconstructed in `assemble` via
`effective_direction` (own direction or inherited), so the flat engine's
cross-boundary waypoints — which M4 discarded — are no longer needed. 5 new
layout tests (frame connection points, two-frame edges, direction preserved
with frame points, stub-doesn't-cross-sibling, nested-frame crossings);
`snapshots/infra.svg` and `snapshots/subdirection.svg` regenerated. (Segments
were diagonal until M7 made them orthogonal.)

**Follow-up — around-the-frame routing for a deep target (post-M7.5).** The
straight within-frame target stub pierces the frame's internal content when
the target node sits beyond other members along the frame's internal flow axis
— e.g. `orders --> db` in `examples/subdirection.mmd`: the Storage subgraph is
top-down, entered from the top, but Database is its *bottom* node with Cache
above it, so the stub ran straight through Cache and then exactly overlapped
the Cache→Database edge. `cross_boundary_path` now detects this (the stub
segment from the frame's facing-side port to the target's same-side port
crosses a sibling of the target inside its frame) and, for the common
single-target-frame / single-or-zero-source-frame case, reroutes via
`try_around_target_route`: the edge runs from the source node, around the
*outside* of the target frame, and into the target from a side perpendicular
to the flow axis — so it clears the sibling and stays distinct from the
internal edge it overlapped. It prefers the side toward the source (a clean L
when the source node is already clear of the frame's cross-span; a gap-jog when
the source sits above the frame within its cross-span), falls back to the far
side if a sibling blocks the near perpendicular entry, and keeps the straight
stub if both sides are blocked or the nesting is deeper than one frame. The
groups' internal layouts stay untouched (the M5 headline guarantee holds).
`assemble` now passes each subgraph's immediate children (node rects + nested
frame rects) to `cross_boundary_path` for the pierce test. 2 new layout tests
(`cross_boundary_edge_routes_around_frame_when_target_is_deep` on the
subdirection sample, and the source-above-frame gap-jog case);
`snapshots/subdirection.svg` regenerated — only the `orders→db` polyline
changed (it now runs down beside Storage and turns into Database's right
side instead of crossing Cache and overlapping the Cache→Database edge).

**Follow-up — node-aligned frame connection points (post-M7.5).** A
cross-boundary edge's representative port on a subgraph frame used to sit at
the frame's *center* on its facing side. That made every edge entering a
subgraph converge on the same midpoint — so two edges into one group read as
both sources reaching both members — and made a within-frame stub jog from
the node's coordinate up to the frame-center coordinate and back (the
`prv-clb --> tke` edge in a nested left-right diagram jogged up to the
subgraph's center-y and back down). `cross_boundary_path` now places each rep
port at the **endpoint node's** cross-coordinate on its frame's facing side
(`port_at_cross`), so a within-frame stub runs straight along the node's own
axis and edges enter a subgraph lined up with their target. The one
complication is the frame's (top-left) title: a straight stub at the node's
`x` would cross the title text when the node sits under it, so a title detour
(`title_detour` / `title_detour_clear_x`) enters the frame just past the
title's right edge and jogs across to the node below the title — but only for
the common single-frame, top-down, grown case where the `CROSS_FRAME_PAD` band
gives room below the title, and only when a clear entry past the title exists;
otherwise the stub stays straight (crossing a too-wide title no worse than the
old center design did). The M7 `JogBias` near-node mechanism is gone (the
detour places its own jog), so `ortho_chain` always jogs at the segment
midpoint. The sibling-pierce around-route and the M5 headline guarantee are
unchanged. 2 new layout tests (`cross_boundary_edges_enter_subgraph_aligned_
with_target_node` and `cross_boundary_edge_between_aligned_nodes_is_straight`);
`cross_boundary_stub_does_not_cross_sibling` and
`cross_boundary_stub_jog_clears_title_region_and_arrowhead` updated for the
new model; `snapshots/infra.svg`, `snapshots/subdirection.svg`, and
`snapshots/subgraph_style.svg` regenerated — cross-boundary edges no longer
merge at a frame's center (e.g. infra's `lb → api1/api2` and `api1/api2 →
db` now enter/leave the K8s frame at each node's own `x`).

**Blocked by:** M4. **Blocks:** nothing new (M7 already blocked by M5.5).

## ✅ M5.5 — Centered/balanced coordinate assignment

The flat engine's x-step used to left-align ranks (left-justified
barycenter), so chains and groups drifted left and never lined up. It now
**centers** every non-source rank on the centroid of its parents, which is
what makes a layered diagram read as aligned rather than ragged.

- [x] Replace the left-justified iterative-barycenter x-step with centered
      **block** placement: within a rank, nodes keep their left-packed
      spread (order + `NODE_SEP` gaps from crossing minimization); each
      non-source rank is then translated as a block so its center lands on
      the centroid of its distinct parents' centers.
- [x] "Parent" is the *real* ancestor behind an upper neighbor (long edges
      are split into zero-size dummies; walk up that chain), so a node
      reached only via a long edge still centers under the real node that
      originates it, not under an intermediate dummy.
- [x] Processed top-down (parents finalized before children); a per-rank
      uniform shift preserves within-rank order and gaps (no overlaps), and
      cross-rank x-overlap is intentional (ranks are stacked along y).
- [x] Tests: a single-parent rank centers its block under the parent; a
      merge node centers on its parents' midpoint; a diamond is symmetric
      on one axis; and the three infra invariants — Web/LB/K8s share a
      center, Postgres/Redis/Message Queue center as a block under the
      cluster, and Replica centers under Postgres. Snapshots `infra`,
      `diamond`, and `subdirection` regenerated.

Implemented in `src/layout.rs` (`assign_x` plus new `center_blocks` /
`real_ancestor`), replacing the old `place_layer_left` barycenter. It is a
contained change to the flat engine, so it applies at **every** level (the
top level and each subgraph) via the existing compound recursion — no
compound-layout or cross-boundary work needed. This is the most visible
quality win since M2: `snapshots/infra.svg` now has a straight
Web→Load Balancer→Kubernetes Cluster spine, with the data tier centered
beneath the cluster and Replica hanging straight off Postgres. 6 new
alignment tests.

**Blocked by:** M2 (the flat engine). **Independent of:** M5, M6.
**Blocks:** M7 (orthogonal edges only look clean once nodes line up).
## ✅ M6 — Render shapes, styles, and colors (already parsed)

Most of this was already parsed in M1; this milestone was purely rendering
work once the SVG pipeline existed.

- [x] Cylinder shape.
- [x] Edge line styles: `dotted`, `dashed`, `thick`.
- [x] Node `color` (stroke) and `fill` (interior); edge `color`.
- [x] Edge labels (positioned along the edge).

Implemented in `src/render/svg.rs`: the `cylinder` shape draws as a body
`<path>` (two vertical sides, a front-bottom arc, and a back-top arc) plus a
full top `<ellipse>` for the lid, sized so the elliptical caps fit exactly
inside the laid-out box rect (cap radius = `0.18·h` clamped to 5–12 px, so
the body always keeps a visible straight section). Edge `style` maps to
stroke attributes — `dotted` → `stroke-dasharray="1 4"` (the shared round
linecaps render that as a row of round dots), `dashed` → `"6 4"`, `thick` →
a doubled `stroke-width` (3 vs the default 1.5); `solid` is the default.
Per-node `color` (stroke) and `fill` (interior), and per-edge `color`, are
carried into the SVG attributes directly. The single arrowhead `<marker>` now
uses `fill="currentColor"`, which inherits the `color` set on each edge's
`<polyline>`, so a colored edge gets a matching arrowhead without a marker
per color. Edge labels are placed at the arc-length midpoint of their
polyline, with a white knockout `<rect>` behind the text so the line reads as
broken behind the label (the classic Graphviz look); labels are drawn after
edges but before nodes, so a node covers any label that strays over it, and
the knockout/text split into two `<g>` groups keeps one label's rect from
covering another's glyphs. Color, fill, and label values are XML-escaped on
the way out (a quoted-string attribute value can legally contain a `"`).
10 new render unit tests + a new `shapes_styles` snapshot exercising the full
M6 surface (cylinders with custom color/fill, all three edge styles, edge
colors, and edge labels); all seven existing snapshots regenerated — the
`currentColor` marker, the per-edge stroke/width/dasharray attributes, and
the new edge-label groups changed every file byte-for-byte. (Diagonal edge
segments came from the M3/M5 router until M7 made them orthogonal.)

**Blocked by:** M3. Can interleave with M4/M5.

## ✅ M7 — Orthogonal edge routing

Upgrade edges from diagonal to right-angle routing typical of infrastructure
diagrams.

- [x] Orthogonal router producing bend points.
- [x] Arrowhead orientation at segment end.
- [x] Snapshot tests updated.

Implemented in `src/layout.rs` (`ortho_chain` plus `FlowAxis` / `cross_flow`
/ `with_flow` / `jog_flow` / `JogBias`, applied in `assemble`): every edge —
direct or cross-boundary — is finally passed through one orthogonalizer that
turns each diagonal segment into a right-angle "Z" — along the edge's flow
axis (the rank axis in page space: vertical for top-down / bottom-up,
horizontal for left-right / right-left) to the segment's flow midpoint,
across to the target's cross coordinate, then along the flow axis to the
target. The flow axis is read per edge from its LCA-level effective
direction, so an edge inside a left-right subgraph jogs horizontally even in
a top-down diagram. The perpendicular jog lands between the two waypoints'
flow coordinates — in an inter-rank `RANK_GAP` or a frame's padding — so it
stays clear of node interiors and the M5 "stub does not cross a sibling"
guarantee holds. Direct edges keep the flat engine's waypoints (dummy
centers and frame connection points are preserved; collinear extras render
the same straight line), so the M2 long-edge and M5 nested-frame tests still
see their waypoints. The one exception is a within-frame stub that crosses a
titled frame's TOP side: a midpoint jog would sit on the title text, so such
stubs instead jog a fixed `STUB_JOG_CLEARANCE` (12 px — sized to exceed both
the renderer's `ARROW_BACKOFF` of 4 px so the arrowhead tip still lands on
the node boundary, and the arrowhead's `markerHeight` of 10 px so the
horizontal approach clears the arrowhead body rather than cutting into its
side) above the node, in the clear padding below the title. That padding is
created by `CROSS_FRAME_PAD`: a subgraph that a cross-boundary edge reaches
through (to an *immediate* child) grows by one subgraph-title font height on
top and bottom, so the 12-px jog lands in the grown band below the title
text rather than on it; a subgraph with no such edge keeps the default frame
geometry. `cross_boundary_path` routes the source stub (`NearStart`), the
LCA segment (`Mid`), and the target stub (`NearEnd`) as separate orthogonal
chains. The arrowhead keeps its `orient="auto"` marker, which the now
axis-aligned final segment orients along a clean cardinal direction, so the
arrow points straight at the target — no renderer change was needed. 9 new
tests (orthogonality over direct/long/fork/merge/cross-boundary/cycled
diagrams, bends on forks, endpoints still on node bounds, a cardinal final
segment, the near-node titled-frame stub jog, SVG-output orthogonality +
`orient="auto"`, the cross-boundary frame growth, and the stub jog clearing
the title region and arrowhead); all eight snapshots regenerated — every
edge is now a right-angle route (`diamond` and `shapes_styles` gained clean
Z-bends; `infra`'s `lb → api1/2` stubs jog below the "Kubernetes Cluster"
title instead of crossing it, and the K8s frame grew to give them room).

**Blocked by:** M3, M5.5. **Independent of:** M4/M5; done after M6.

## ✅ M7.5 — Subgraph color, fill, and line (border) style; text color

- [x] Subgraph color attribute (color for the line (border))
- [x] Subgraph fill attribute (color of the fill (background) of the subgraph)
- [x] Subgraph line style (dashed, dotted, thick, etc.)
- [x] For all text: text attribute (color of the text)

Implemented in `src/resolve.rs` + `src/render/svg.rs` (+ new fields on the
resolved `Subgraph`/`Node`/`Edge` in `src/ast.rs`): the resolver now accepts
four subgraph style attributes — `color` (border), `fill` (background),
`line` (border style: `solid`/`dotted`/`dashed`/`thick`, supplied as a
quoted value since the subgraph header has no style keyword slot), and
`text` (title color) — and a `text` text-color attribute on nodes and
edges. `line`'s value is validated against the `Style` set (an unknown
value like `line="wavy"` is a resolve error); node `text` follows the same
redeclaration-consistency rule as `color`/`fill`. The renderer now zips
`diagram.subgraphs` with `layout.subgraphs` (the index-correspondence
invariant) to read a frame's `color`/`fill`/`line`/`text` from the resolved
`Subgraph` and its geometry from `SubgraphRect` — the same split it already
uses for nodes and edges — so `SubgraphRect` is now geometry-only (its
`title` field moved to `Subgraph`, which already carried it). A frame with
no style attributes renders byte-identically to the pre-M7.5 output: the
`<g>` keeps the default frame stroke/width, each `<rect>` overrides with
its own (defaulting to those same constants), and a title with no `text`
inherits the group's default fill. `line` maps via a `frame_stroke` helper
mirroring `edge_stroke` (`dotted`/`dashed` reuse the same dash patterns;
`thick` doubles the frame border width, mirroring the edge convention);
`fill` defaults to `none` so a default subgraph stays transparent and edges
behind it stay visible. The `text` attribute sets a per-element `fill` on
the node label, the edge-label `<text>`, and the subgraph-title `<text>`,
each only when present (default text keeps the existing group/INK color, so
unstyled diagrams are unchanged). 19 new tests (subgraph attrs parsed + each
`line` style + invalid `line` value + unknown/duplicate subgraph attrs +
node/edge `text` + `text` redeclaration consistency + render checks for
every new attribute + a `subgraph_style` snapshot); all eight existing
snapshots regenerated and verified byte-identical (only a new
`subgraph_style.svg` was added) — subgraph styling is render-only, so layout
geometry is untouched.

**Blocked by:** M3 (rendering), M6 (attribute pipeline). **Independent of:**
M7 (orthogonal routing).

## ⬜ M8 — CLI polish and extras

- [ ] `--direction` flag to override the header.
- [ ] Theme/style presets (reusable style classes — grammar slot is reserved).
- [ ] Error message quality pass (context spans, suggestions).
- [ ] README usage section + sample SVG committed and embedded.

**Blocked by:** M3. Otherwise incremental.

---

### Dependency graph

```
M0 ── M1 ── M2 ── M3 ── M4 ── M5
              │   │      └── M6 (interleaves)
              │   └──────── M8
              └── M5.5 ── M7
```
