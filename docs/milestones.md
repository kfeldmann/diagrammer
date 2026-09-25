# Milestones

Tracking doc for diagrammer. The grammar is specified in
[`grammar.md`](grammar.md); this file records what's done, what's next, and
how the remaining work depends on itself.

**Status legend:** ✅ done · ▶ next · ⬜ planned

**Current position:** M0–M7.5, M9, M10, and M11 are complete. **M11.5 (routing-layer refactor; absorbs M12) is next**, then M13; M8 (CLI polish) is deferred until after.

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

## ⬜ M8 — CLI polish and extras *(deferred until after M13)*

- [ ] `--direction` flag to override the header.
- [ ] Theme/style presets (reusable style classes — grammar slot is reserved).
- [ ] Error message quality pass (context spans, suggestions).
- [ ] Input-robustness guards: deeply nested subgraphs currently recurse on
      the native stack in both the parser and the compound layout — a
      pathologically nested input stack-overflows instead of erroring
      cleanly. Cap or depth-proof both.
- [ ] README usage section + sample SVG committed and embedded.

**Blocked by:** M3. Otherwise incremental.

## ✅ M9 — Edge separation & obstacle-aware LCA routing

The M7 per-edge midpoint-jog orthogonalizer had two failure modes that this
milestone eliminates: (1) an edge passing *through* a node — the LCA-level
segment of a cross-boundary edge could jog through a peer node sitting between
its endpoints (e.g. `User --> VPN` whose jog crossed `prd`, making it read as
if `prd` connected to `VPN`); (2) parallel edges laying *on top of* each other
— several edges from one source to one target subgraph shared the source's
single port and the same jog lane, so their trunks overlapped and their labels
collided (e.g. the five `VPN --> cvm` edges, and `lb --> api1/api2` in
`infra`). Crossing is allowed; laying on top, and passing behind a node, are
not.

- [x] **Port separation** — a node side carrying more than one edge fans the
      ports across the side (each placed at the *other* endpoint's
      cross-coordinate projected onto the side, then nudged apart to a minimum
      spacing), so edges no longer all leave/enter at the centre. Single-edge
      sides keep the centre (the prior, tested behaviour).
- [x] **Lane separation** — cross-boundary LCA segments sharing a source,
      source side, and gap run at distinct jog flow-coordinates in that gap,
      fanning without overlapping. (Grouped by the shared gap, not the target,
      so same-gap edges like `api2 --> {db, cache, queue}` share lanes too.)
- [x] **Obstacle avoidance** — when an LCA segment's jog would cross a peer
      node at its level, it reroutes around it through a rectilinear track
      graph (Dijkstra over obstacle-edge tracks, with a bend penalty for
      fewer corners), so no edge passes through a node. Lanes are kept clear of
      obstacles that intrude into the gap, so the router never falls back to a
      colliding midpoint.
- [x] Tests: no two edges share a collinear overlapping segment and no edge
      segment passes through any non-endpoint node, asserted on `infra`,
      `subdirection`, and the new `finance` sample; a cross-boundary LCA
      around-a-peer-node test; orthogonality preserved.

Implemented in `src/layout.rs` (M9 section after `orthogonalize`; plus changes
to `cross_boundary_path`, which now returns a `CrossPath` of within-frame stubs
plus an LCA segment for the caller to route, and to `assemble`, which runs a
port-separation pre-pass, a lane-separation pre-pass, and a per-level obstacle
field). Within-frame stubs keep the M5/M7 frame-aware logic unchanged (title
detours, around-target-frame routes, nested-frame crossings); only the
inter-representative LCA segment and direct-edge endpoints are rerouted, so the
M5 headline guarantee (a subgraph's internal layout is never disturbed by a
crossing edge) holds. New sample `examples/finance.mmd` + snapshot
`snapshots/finance.svg`; `infra`, `diamond`, `cycle`, and `shapes_styles`
snapshots regenerated (forks and 2-cycles now fan ports — e.g. an `a <-> b`
cycle renders as two distinct parallel lines instead of one overlapping).

**Known limitation:** edge labels are placed by the renderer at the midpoint of
their polyline's longest segment; in a crowded gap (many parallel edges in one
`RANK_GAP`) a long multi-line label can still crowd a neighbour (e.g.
`vpn --> cvm1`'s two-line port list vs `vpn --> cvm3` in the finance diagram).
The edges themselves are fully separated; smarter label placement is
addressed in M13.

**Blocked by:** M7 (orthogonal routing), M5 (cross-boundary structure).
**Independent of:** M8.

---

## ✅ M10 — Title-detour robustness

The M5.5 title detour had two verified failure modes that grow worse with
longer subgraph titles; both are fixed.

- [x] **Entry gap eaten by viewer font fallback.** The detour used to enter
      exactly `TITLE_CLEAR_GAP` (6 px) past the title's *measured* right
      edge. Text is measured with the embedded DejaVu Sans (determinism
      invariant), but SVGs are viewed in Firefox / GitHub READMEs / Confluence
      where DejaVu is often absent and the font stack falls back to a
      different-width face — the drawn title extends past the measured end
      and eats the gap, proportionally to title length. The right-edge
      clearance is now `title_clear_gap(title_w)` = base 6 px + a fixed 12 px
      bump + 0.1 px per pixel of measured title width (`TITLE_FALLBACK_PAD` /
      `TITLE_FALLBACK_PER_W`), since the measurement font cannot change.
- [x] **No room past the title on title-sized frames.** When the title
      determines the frame width (`title_w + 2·FRAME_TITLE_X`), the right
      margin past the title was only `FRAME_TITLE_X` (10 px), so the clamped
      entry fell inside the title and the detour gave up (straight stub
      through the title). Fixed by mirroring the `CROSS_FRAME_PAD` trick
      horizontally: when a cross-boundary edge reaches an immediate child
      that sits under the title's right half and the natural width leaves no
      clear band past the title, `layout_level` grows the frame's width to
      `title_w + FRAME_TITLE_X + gap + FRAME_PAD_X` — sideways growth keeps
      the children anchored at the left/top insets, so the group's internal
      arrangement is untouched (the M5 headline guarantee holds). Like
      `CROSS_FRAME_PAD` it over-approximates (the entry side is not yet known
      at frame-sizing time), so a frame may widen for a detour that never
      arises.
- [x] **Left-side detour.** `title_detour_clear_x` now considers two
      entries and takes the one nearer the node: past the title's right edge
      (fallback-safe gap), or just left of the title's first glyph
      (`title_x0 - TITLE_CLEAR_GAP`) when the node sits under the title's
      left half. The left edge is safe with the small base gap because the
      renderer anchors the title at `FRAME_TITLE_X` — no font fallback can
      widen it leftward — so the title-sized case with a left-half child
      needs no growth at all; the right-half case is what triggers it.

Implemented in `src/layout.rs`: the gap function `title_clear_gap` and the
rewritten two-entry `title_detour_clear_x` (prefer the nearer side, fall
back to the other, give up only if neither fits — the growth pass makes the
give-up unreachable for cross-boundary endpoints), the `cross_boundary_subs`
refactor into `cross_boundary_reach` (now also returns the cross-boundary
*endpoint nodes*, threaded through `layout_level` as `cross_nodes`), and the
horizontal growth pass in `layout_level`'s child-frame sizing. 4 new tests
(`title_clear_gap_grows_with_title_length`,
`title_sized_frame_left_detour_when_node_sits_left_of_title`,
`title_sized_frame_grows_width_for_right_half_detour`,
`title_detour_entry_clears_measured_title_by_the_fallback_gap` — the latter
three assert the title rect is never crossed);
`snapshots/infra.svg`, `snapshots/subdirection.svg`, and
`snapshots/subgraph_style.svg` regenerated — the only change is that title
-detour entries sit a fallback-safe gap further right of their titles (e.g.
infra's `lb → api1` now enters the K8s frame at x = title-right + 27.8 px
instead of + 6 px); no frame widths changed in the samples (none of them has
a title-sized cross-reached frame).

**Blocked by:** nothing (self-contained bug fix). **Blocks:** nothing, but it
establishes the "grow the frame to make routing room" pattern M11–M13 may
reuse.

## ✅ M11 — Edge side attributes (`from=` / `to=`)

Let an edge request which page-space side of each node it connects to:
`A -- from="right" to="top" --> B` leaves A from its right side and enters B
at its top. The algorithm already chooses sides implicitly; this makes them
explicit to mitigate ugly routing.

- [x] Grammar + parser: `from` and `to` recognized as edge-body attributes;
      values `top | bottom | left | right`; unknown values are resolve
      errors (strict-attribute convention).
- [x] Sides are **page-space** (as drawn), unambiguous regardless of the
      diagram's or any subgraph's direction.
- [x] Resolved `Edge` carries `from_side` / `to_side`; layout honors them at
      the two port-decision points — the flat engine's `edge_waypoints`
      (direct edges) and `sides_along` (cross-boundary LCA segments).
- [x] **Literal semantics** (decided): the forced side is **always honored** —
      the port lands on the requested side, with no automatic reversion to the
      algorithm's choice. The attribute is an override; silently
      second-guessing it would leave the user unable to tell whether it did
      anything. A side that contradicts the approach direction is routed
      around (cf. `try_around_target_route`); the result may be ugly, and
      that is the user's cue to change or drop the attribute. `grammar.md`
      will document this: *the attribute is honored literally; contradictory
      choices produce ugly (but valid, non-overlapping) routes.*
- [x] **Routing must always succeed.** Forced sides can push routes outside
      the current content bounds (e.g. an excursion around a wide subgraph);
      the track graph needs unbounded escape corridors and the viewBox grows
      to include them. What remains guaranteed even for contradictory
      choices: orthogonal segments, no passing through node interiors, no two
      edges laying collinearly on top of each other (M9 invariants), and port
      separation still fans multiple edges sharing a forced side. Note the
      existing termination ladder in `route_lca` (simple Z → mid jog →
      track route → last-resort possibly-crossing Z) already ends in a
      route that keeps the ports — the right shape for literal semantics;
      keep that ladder (bounded, never loops) and make sure forced-side
      routes use it.
- [x] Self-loops with forced sides (`A --> A` with `from`/`to`) need real
      loop routing, not the current fixed below-node stub.
- [x] Tests: each forced side in each direction; a contradictory side routes
      around (not fall back); forced-side fan; forced-side excursion beyond
      the content bounds (viewBox grows); self-loop with forced sides;
      snapshots regenerated.

Implemented across `src/resolve.rs` (`parse_edge_side` + `Edge.from_side`/
`to_side` on the resolved edge) and `src/layout.rs`. Each forced port first
**escapes** outward along its side's normal (`SIDE_ESCAPE`), then the
`route_lca` ladder routes between the escaped points with the endpoint nodes
and peer-edge segments (`SEGMENT_OBSTACLE_PAD`-inflated) as obstacles, so the
route cannot cut back through either box nor stack collinearly on a
neighbor. Three M11 routers sit on top of that: `forced_direct_path` for
direct edges, `self_loop_path` for forced self-loops (same sides → a
rectangular bump; adjacent/opposite sides → corner wraps), and the
within-frame stub avoidance (`force_stub_around_siblings`); already-routed
edges become obstacles via `segment_rect`, so two forced excursions in one
gap cannot lie collinearly on top of each other. A
canvas-growth pass in `assemble` grows — and, for negative excursions,
translates — the canvas so every drawn point lands inside the viewBox.

One latent M9 bug surfaced here and was fixed: `track_route` clear-checked
its candidate segments against **un-inflated** obstacle rects while its
caller (`route_clear`) inflates them by `ROUTE_PAD`; with node-rect-only
obstacles the well-separated rects never tripped this, but the M11 peer-
segment rects sit close to nodes, so `track_route` returned routes its own
caller rejected and the blocked simple Z was used as the last resort —
producing exactly the collinear stacking M11 forbids. `track_route` now
clear-checks against `ROUTE_PAD`-inflated rects, matching `route_clear`.

`examples/sides.mmd` is the canonical M11 sample (agreeable + contradictory
sides, a forced self-loop, a forced cross-boundary edge in a left-right
subgraph), with a golden `sides.svg` snapshot alongside the others.

**Blocked by:** M9. **Blocks:** M11.5, M13 (they build on the final geometry so
the cosmetic work is done once).

## ▶ M11.5 — Routing-layer refactor: one obstacle world, one routing regime

Review of the routing layer found it structurally sound (the compound/LCA
decomposition, the `(cross, flow)` vocabulary, the `route_lca` ladder, and the
invariant tests are the part that works and everything downstream assumes it)
but past the point where per-case tweaking is safe. The routing regime differs
by edge kind, with different guarantees each: **direct edges** keep the flat
engine's waypoints and orthogonalize blind (no obstacle check at all — only
forced sides get the ladder — so a non-forced direct edge's midpoint jog can
still pass through a peer node); **cross-boundary edges** route obstacle-aware
through the M9 ladder against node rects only; **forced edges** additionally
treat already-routed edges as obstacles, making route quality depend on
declaration order. Meanwhile the special-case interlock conditions
(`is_around_target` port-separation exclusion, the around-target / title-detour
/ forced-stub gates) are scattered across `assemble` and
`cross_boundary_path`, and `assemble` (~540 lines) derives the same per-edge
context (forced sides, chains, rep rects) twice — once in the pre-pass, once in
the routing loop — with agreement maintained by hand.

This milestone restructures the routing layer into **one regime**: the same
router and the same obstacle world for every edge kind, with edge-type
knowledge confined to which obstacles and candidate routes each edge feeds it.
It **absorbs M12** (edges avoid frame outlines), whose scope is step 1 below.
Everything that works stays: the compound/LCA decomposition, the `(cross,
flow)` vocabulary, the `route_lca` ladder, and all existing invariants.

- [ ] **One obstacle world (absorbs M12).** A single obstacle model used by
      every router: node rects; subgraph **frame borders** as thin obstacles
      that may only be crossed at designated connection points (collinear
      overlap with a border barred except at an entry/exit crossing — a
      frame's interior stays legal, since crossing a nested frame is
      intentional and recorded as a waypoint); and **previously routed edge
      segments** (`segment_rect`-inflated) as obstacles for *all* edges, not
      only forced ones. `route_clear` / `track_route` clear-check against this
      one (consistently inflated) world — eliminating the class of latent bug
      M11 already hit once (mismatched inflation between the two checks).
- [ ] **Every edge routes through the ladder.** Non-forced direct edges run the
      same `route_lca` ladder (simple Z at the lane → mid jog → track route →
      last-resort Z) instead of blind `orthogonalize`, closing the
      pass-through-a-node hole; the simple Z remains the fast path when the
      obstacle set is empty or the lane is clear.
- [ ] **Deterministic routing order.** Edges are routed in one pass in a
      deterministic priority (forced edges first, then declaration order), so
      peer-segment obstacles no longer depend on an edge happening to be
      forced.
- [ ] **RouteContext hoist.** Extract a per-edge `RouteContext` built **once** —
      endpoint rects, chains, rep rects, effective sides (implicit vs forced
      resolved in *one* place), port-separation results, lane assignments,
      level obstacles — and rebuild `assemble`'s edge loop to: build context →
      generate candidate routes (straight Z / around-target / title-detour
      stub / forced ladder / self-loop) → common clearance check → accept.
      Each special-case *gate* moves from scattered booleans into its
      candidate generator (each generator decides whether it applies); a
      single dispatch point decides. The special cases keep their fallbacks —
      the point is one dispatch, not fewer cases.
- [ ] **No invariant regression.** The M5 headline guarantee (crossing edges
      never disturb group internal layout), the M9 invariants (no collinear
      stacking, no node pass-through, orthogonality), and the M11 literal
      forced-side semantics all keep passing. The existing invariant test
      suite is the definition of success; snapshots regenerate only where the
      unified routing actually improves a route (e.g. a direct-edge jog that
      used to cross a peer node).
- [ ] **Decision point — global router? (evaluate, do not build).** After steps
      1–2, evaluate on the samples plus adversarial cases: if the unified
      regime still needs per-case fallbacks to satisfy the invariants, the
      next move is a global routing pass (all edges against a shared occupancy
      grid, deterministic priority) — not another per-case patch. Record the
      verdict in this section; explicitly out of scope to build here.

**Blocked by:** M11. **Blocks:** M13 (they build on the final geometry so the
cosmetic work is done once).

## ⬜ M13 — Label placement engine + parallel-edge spacing

Edge labels are placed blind at the longest segment's midpoint (M9's known
limitation); parallel edges sit close enough that a label's white knockout
covers a neighboring edge.

- [ ] **Move label anchors into layout** (decided): `assemble` computes each
      label's anchor — it knows all geometry: obstacles, lanes, frames, other
      edges — and stores it on `EdgePath` (a per-edge field, so the
      index-correspondence invariant is safe); the renderer is slimmed to
      draw the knockout rect + text at the given anchor.
- [ ] Placement algorithm: candidate positions along each polyline (offsets
      along segments), scored against other edges' segments, other labels'
      rects, node rects, and frame borders/titles; deterministic greedy pass
      with a refinement pass (no randomness, fixed iteration order).
- [ ] **Label-aware parallel spacing**: lane/fan assignment informed by label
      sizes so a labeled edge's knockout cannot cover a neighbor; bump
      `FAN_SEP` / `LANE_INSET` / `LABEL_PAD` as needed.

**Blocked by:** M11.5 (final geometry + unified obstacle model). Resolves M9's
known limitation.

---

### Dependency graph

```
M0 ── M1 ── M2 ── M3 ── M4 ── M5 ── M9 ── M10 ── M11 ── M11.5 ── M13
              │   │      └── M6 (interleaves)        │
              │   └──────── M8 (deferred) ───────────┘
              └── M5.5 ── M7 ─┘
```
