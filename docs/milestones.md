# Milestones

Tracking doc for diagrammer. The grammar is specified in
[`grammar.md`](grammar.md); this file records what's done, what's next, and
how the remaining work depends on itself.

**Status legend:** ✅done · ▶ next · ☐ planned

**Current position:** M0–M7.5, M9–M15, M16, M17, M18, and M19 are
complete (M12 was absorbed by M11.5), plus the post-M13 hardening pass
below and the post-M17 routing hardening (escape-stub spikes) recorded
under M17. M20.1 (whole-path maze routing: foundation), M20.2 (whole-path
search for direct internal edges), M20.3 (whole-path search for
cross-boundary edges; rep model deleted), and M20.4 (frame-crossing fan;
pockets and the repair loop re-grounded) are complete.
**M8 (CLI polish) and the M20.5–M20.6 routing migration remain open.**

---

## ✅ M0 — Grammar spec

Define the v0 input language: readable, line-oriented, Mermaid-inspired but
not backward-compatible. See [`grammar.md`](grammar.md).

- [x] Header + direction
- [x] Nodes (bare id / `id "label"`, `: shape`, attrs)
- [x] Edges (`-->`, `-- style "label" attrs -->`)
- [x] Groups (visual grouping; direction slot reserved)
- [x] Comments (`#`), quoted strings with escapes
- [x] Decisions locked: `diagram` header; full-word directions; `-- "label" -->`
      edge labels; implicit node declarations; `#` comments; strict unknown
      attributes; styles `solid | dotted | dashed | thick`.

## ✅ M1 — Parser + semantic validation

Parse the full v0 language into a raw AST and validate it into a resolved
diagram. Layout and rendering are stubbed (`todo!()`).

- [x] Lexer: whitespace/comments, identifiers (hyphen rule so `A-->B` works),
      quoted strings with `\"`/`\\` escapes, line endings.
- [x] Parser (winnow): diagram header, nodes, edge chains, groups (nested),
      attributes. Errors carry a byte offset + context labels → 1-based line
      numbers.
- [x] Resolver: dedup nodes, label/shape/attribute consistency on
      redeclaration, strict attribute validation (unknown & duplicate → error),
      group membership (a node may belong to at most one group), reject
      per-group direction.
- [x] Cross-boundary edges do **not** relocate a node (the Mermaid/dagre
      behavior we want to escape). Occurrences inside a `group` body set
      membership — standalone declarations and edge-chain endpoints alike
      (an edge endpoint naming a node already placed in another group is a
      cross-boundary reference and leaves it put); occurrences at top level
      never move a node.
- [x] Minimal CLI: `diagrammer <input.dgmr>` validates and prints a summary
      (`ok: N nodes, M edges, K groups (direction ...)`).
- [x] Test suite: 20 tests covering parse, resolve, and error cases.

---

## ✅ M2 — Layout engine (Sugiyama-style)

Produce coordinates and a ranking for a single flat graph under one global
direction. No groups yet.

- [x] **Text measurement** (precise, against baked DejaVu metrics): node
      label → pixel
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
against baked metrics generated from the DejaVu Sans Regular embedded via
the `dejavu` crate (deterministic across machines). 16 layout tests + 4 measurement tests.

**Blocked by:** nothing. **Blocks:** M3, M4.

## ✅ M3 — SVG rendering + CLI output

Emit self-contained static SVG from a `Layout`. Wire the CLI to write a file.
GitHub-renderable (no scripts/external refs).

- [x] Box rendering (rounded rect + centered label).
- [x] **Diagonal/direct edges** to start (straight lines), arrowheads via
      `<marker>`.
- [x] SVG document wrapper (viewBox sized to content).
- [x] CLI: `diagrammer <input.dgmr> -o <output.svg>` (and read direction/output
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

## ✅ M4 — Groups with per-group direction (headline feature)

Extend the layout to model compound nodes (groups) and honor a per-group
direction. This is the core reason the tool exists.

- [x] Compound layout: groups contain child nodes/groups; parents size
      to fit children.
- [x] Honor `group <direction>` (lift the M1 rejection into layout).
- [x] Group frame rendering (labeled border around members).
- [x] Tests: same graph laid out with different per-group directions.

Implemented in `src/layout.rs` + `src/render/svg.rs` (+ containment fields in
`src/ast.rs` / `src/resolve.rs`): the flat M2 engine was extracted into a
parameterized `layout_flat(items, edges, direction, margin)` and driven
**recursively** per level. Each level (the top-level diagram or a group)
lays out its direct-child real nodes plus its direct-child groups as opaque
compound boxes; a group box is sized to fit its own recursively-laid-out
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

Edges crossing group boundaries must route without collapsing the group's
internal layout (the Mermaid failure mode).

- [x] Edge routing aware of compound boundaries.
- [x] Connection points on group frames.
- [x] Tests: cross-boundary edges preserve per-group direction.

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
the M4 recursion), so a left-right group chain stays horizontal even while
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
— e.g. `orders --> db` in `examples/subdirection.dgmr`: the Storage group is
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
`assemble` now passes each group's immediate children (node rects + nested
frame rects) to `cross_boundary_path` for the pierce test. 2 new layout tests
(`cross_boundary_edge_routes_around_frame_when_target_is_deep` on the
subdirection sample, and the source-above-frame gap-jog case);
`snapshots/subdirection.svg` regenerated — only the `orders→db` polyline
changed (it now runs down beside Storage and turns into Database's right
side instead of crossing Cache and overlapping the Cache→Database edge).

**Follow-up — node-aligned frame connection points (post-M7.5).** A
cross-boundary edge's representative port on a group frame used to sit at
the frame's *center* on its facing side. That made every edge entering a
group converge on the same midpoint — so two edges into one group read as
both sources reaching both members — and made a within-frame stub jog from
the node's coordinate up to the frame-center coordinate and back (the
`prv-clb --> tke` edge in a nested left-right diagram jogged up to the
group's center-y and back down). `cross_boundary_path` now places each rep
port at the **endpoint node's** cross-coordinate on its frame's facing side
(`port_at_cross`), so a within-frame stub runs straight along the node's own
axis and edges enter a group lined up with their target. The one
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
unchanged. 2 new layout tests (`cross_boundary_edges_enter_group_aligned_
with_target_node` and `cross_boundary_edge_between_aligned_nodes_is_straight`);
`cross_boundary_stub_does_not_cross_sibling` and
`cross_boundary_stub_jog_clears_title_region_and_arrowhead` updated for the
new model; `snapshots/infra.svg`, `snapshots/subdirection.svg`, and
`snapshots/group_style.svg` regenerated — cross-boundary edges no longer
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
top level and each group) via the existing compound recursion — no
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
direction, so an edge inside a left-right group jogs horizontally even in
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
created by `CROSS_FRAME_PAD`: a group that a cross-boundary edge reaches
through (to an *immediate* child) grows by one group-title font height on
top and bottom, so the 12-px jog lands in the grown band below the title
text rather than on it; a group with no such edge keeps the default frame
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

## ✅ M7.5 — Group color, fill, and line (border) style; text color

- [x] Group color attribute (color for the line (border))
- [x] Group fill attribute (color of the fill (background) of the group)
- [x] Group line style (dashed, dotted, thick, etc.)
- [x] For all text: text attribute (color of the text)

Implemented in `src/resolve.rs` + `src/render/svg.rs` (+ new fields on the
resolved `Group`/`Node`/`Edge` in `src/ast.rs`): the resolver now accepts
four group style attributes — `color` (border), `fill` (background),
`line` (border style: `solid`/`dotted`/`dashed`/`thick`, supplied as a
quoted value since the group header has no style keyword slot), and
`text` (title color) — and a `text` text-color attribute on nodes and
edges. `line`'s value is validated against the `Style` set (an unknown
value like `line="wavy"` is a resolve error); node `text` follows the same
redeclaration-consistency rule as `color`/`fill`. The renderer now zips
`diagram.groups` with `layout.groups` (the index-correspondence
invariant) to read a frame's `color`/`fill`/`line`/`text` from the resolved
`Group` and its geometry from `GroupRect` — the same split it already
uses for nodes and edges — so `GroupRect` is now geometry-only (its
`title` field moved to `Group`, which already carried it). A frame with
no style attributes renders byte-identically to the pre-M7.5 output: the
`<g>` keeps the default frame stroke/width, each `<rect>` overrides with
its own (defaulting to those same constants), and a title with no `text`
inherits the group's default fill. `line` maps via a `frame_stroke` helper
mirroring `edge_stroke` (`dotted`/`dashed` reuse the same dash patterns;
`thick` doubles the frame border width, mirroring the edge convention);
`fill` originally defaulted to `none` so a default group stayed
transparent (edges behind it visible) — the default-palette change later
switched the frame defaults to `fill="#f2f8f4"` / `color="#88BDA4"` and the
node defaults to `fill="#c0e1fc"` / `color="#2196F3"` (see `grammar.md`), with
explicit `fill="none"` still available for transparency. The `text`
attribute sets a per-element `fill` on
the node label, the edge-label `<text>`, and the group-title `<text>`,
each only when present (default text keeps the existing group/INK color, so
unstyled diagrams are unchanged). 19 new tests (group attrs parsed + each
`line` style + invalid `line` value + unknown/duplicate group attrs +
node/edge `text` + `text` redeclaration consistency + render checks for
every new attribute + a `group_style` snapshot); all eight existing
snapshots regenerated and verified byte-identical (only a new
`group_style.svg` was added) — group styling is render-only, so layout
geometry is untouched.

**Blocked by:** M3 (rendering), M6 (attribute pipeline). **Independent of:**
M7 (orthogonal routing).

## ⬜ M8 — CLI polish and extras *(the remaining milestone)*

- [ ] `--direction` flag to override the header.
- [ ] Theme/style presets (reusable style classes — grammar slot is reserved).
- [ ] Error message quality pass (context spans, suggestions).
- [ ] Input-robustness guards: deeply nested groups currently recurse on
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
— several edges from one source to one target group shared the source's
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
M5 headline guarantee (a group's internal layout is never disturbed by a
crossing edge) holds. New sample `examples/finance.dgmr` + snapshot
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
longer group titles; both are fixed.

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
`snapshots/group_style.svg` regenerated — the only change is that title
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
      diagram's or any group's direction.
- [x] Resolved `Edge` carries `from_side` / `to_side`; layout honors them at
      the two port-decision points — the flat engine's `edge_waypoints`
      (direct edges) and `sides_along` (cross-boundary LCA segments).
- [x] **Literal semantics** (decided): the forced side is **always honored** —
      the port lands on the requested side, with no automatic reversion to the
      algorithm's choice. (M14 later narrowed the *scope*: the forced side is
      honored literally at the endpoint **node**; the group frames an edge
      crosses keep their natural sides — see M14.) The attribute is an override; silently
      second-guessing it would leave the user unable to tell whether it did
      anything. A side that contradicts the approach direction is routed
      around (cf. `try_around_target_route`); the result may be ugly, and
      that is the user's cue to change or drop the attribute. `grammar.md`
      will document this: *the attribute is honored literally; contradictory
      choices produce ugly (but valid, non-overlapping) routes.*
- [x] **Routing must always succeed.** Forced sides can push routes outside
      the current content bounds (e.g. an excursion around a wide group);
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
canvas-fit pass in `assemble` translates — and grows the canvas — so every
drawn point lands inside the viewBox with the page `MARGIN` (20 px) clear on
every side (nodes, frames, edge waypoints and label knockouts alike), so a
route that bulges past the margin shifts the drawing instead of touching the
canvas edge.

One latent M9 bug surfaced here and was fixed: `track_route` clear-checked
its candidate segments against **un-inflated** obstacle rects while its
caller (`route_clear`) inflates them by `ROUTE_PAD`; with node-rect-only
obstacles the well-separated rects never tripped this, but the M11 peer-
segment rects sit close to nodes, so `track_route` returned routes its own
caller rejected and the blocked simple Z was used as the last resort —
producing exactly the collinear stacking M11 forbids. `track_route` now
clear-checks against `ROUTE_PAD`-inflated rects, matching `route_clear`.

`examples/sides.dgmr` is the canonical M11 sample (agreeable + contradictory
sides, a forced self-loop, a forced cross-boundary edge in a left-right
group), with a golden `sides.svg` snapshot alongside the others.

**Blocked by:** M9. **Blocks:** M11.5, M13 (they build on the final geometry so
the cosmetic work is done once).

## ✅ M11.5 — Routing-layer refactor: one obstacle world, one routing regime

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

- [x] **One obstacle world (absorbs M12).** A single obstacle model used by
      every router: node rects; group **frame borders** as thin obstacles
      that may only be crossed at designated connection points (collinear
      overlap with a border barred except at an entry/exit crossing — a
      frame's interior stays legal, since crossing a nested frame is
      intentional and recorded as a waypoint); and **previously routed edge
      segments** (`segment_rect`-inflated) as obstacles for *all* edges, not
      only forced ones. `route_clear` / `track_route` clear-check against this
      one (consistently inflated) world — eliminating the class of latent bug
      M11 already hit once (mismatched inflation between the two checks).
- [x] **Every edge routes through the ladder.** Non-forced direct edges run the
      same `route_lca` ladder (simple Z at the lane → mid jog → track route →
      last-resort Z) instead of blind `orthogonalize`, closing the
      pass-through-a-node hole; the simple Z remains the fast path when the
      obstacle set is empty or the lane is clear.
- [x] **Deterministic routing order.** Edges are routed in one pass in a
      deterministic priority (forced edges first, then declaration order), so
      peer-segment obstacles no longer depend on an edge happening to be
      forced.
- [x] **RouteContext hoist.** Extract a per-edge `RouteContext` built **once** —
      endpoint rects, chains, rep rects, effective sides (implicit vs forced
      resolved in *one* place), port-separation results, lane assignments,
      level obstacles — and rebuild `assemble`'s edge loop to: build context →
      generate candidate routes (straight Z / around-target / title-detour
      stub / forced ladder / self-loop) → common clearance check → accept.
      Each special-case *gate* moves from scattered booleans into its
      candidate generator (each generator decides whether it applies); a
      single dispatch point decides. The special cases keep their fallbacks —
      the point is one dispatch, not fewer cases.
- [x] **No invariant regression.** The M5 headline guarantee (crossing edges
      never disturb group internal layout), the M9 invariants (no collinear
      stacking, no node pass-through, orthogonality), and the M11 literal
      forced-side semantics all keep passing. The existing invariant test
      suite is the definition of success; snapshots regenerate only where the
      unified routing actually improves a route (e.g. a direct-edge jog that
      used to cross a peer node).
- [x] **Decision point — global router? (evaluate, do not build).** After steps
      1–2, evaluate on the samples plus adversarial cases: if the unified
      regime still needs per-case fallbacks to satisfy the invariants, the
      next move is a global routing pass (all edges against a shared occupancy
      grid, deterministic priority) — not another per-case patch. Record the
      verdict in this section; explicitly out of scope to build here.

**Verdict (recorded retroactively, post-M13): YES — the criterion is
      met; the next routing move is the global pass, not another patch.** The
      unified regime satisfies the invariants only through per-case fallbacks,
      and they fire on the canonical samples:

1. **The last-resort Z is an invariant-breaking fallback, and it fires
         on a sample.** A sweep of all 11 golden snapshots for edge segments
         cutting node interiors finds exactly one violation:
         `examples/sides.dgmr`, `api -- from="top" --> db` — the
         kept-unconditional ladder fallback draws two segments through `api`,
         against the M11 promise ("routed around … never through a node").
         `bus -- from="right" to="left" --> queue` on the same sample also
         falls through the entire ladder (2 of 7 edges). Structurally, both
         `route_lca` Z rungs are dead for every *contradictory* forced side —
         the escape plus the flow-first Z always re-crosses the source box —
         so such edges are `track_route`-or-bust and the fallback fires
         whenever the world is crowded.
2. **The root cause is a resource conflict the one-pass regime cannot
         arbitrate — and it is a regression the per-case regime caused
         itself.** Both failures seal a forced port's `SIDE_ESCAPE` pocket
         inside the 8 px `SEGMENT_OBSTACLE_PAD` bands of already-routed peer
         segments (margins observed: 0.31 px — `api --> cache`'s vertical run
         vs. `db`'s top-port corridor — and 3 px — the worker self-loop's
         stub); `seg_clear_padded` bars parallel overlap absolutely, so
         `track_route`'s Dijkstra cannot leave the point and returns empty.
         Reverting `SEGMENT_OBSTACLE_PAD` to the pre-hardening 5 px makes the
         edge route exactly as expected (up, left, down into `db`): the
         hardening's parallel-minimum bump, meeting immutable hard bands in a
         declaration-order router, traded the no-node-pass-through invariant
         for the no-parallel-riding one. Two invariants ping-ponging under
         local tweaks is exactly the signal this item was written to catch.
3. **No per-case patch fixes it inside the invariants.** Restoring
         "never through a node" requires moving an already-routed edge
         (rip-up / simultaneous assignment — absent from the architecture) or
         carving new exemptions (port-corridor escapes, pad shrink near
         ports) — more per-case fallbacks, which is what this item rules out.
         Extra ladder rungs (e.g. a cross-at-escape-height Z) fix one
         instance's shape but not the seal. The adversarial shape is trivial:
         any routed L-corner within ~8 px of another edge's port escape seals
         it (the self-loop stub in `sides.dgmr` is exactly this).

So: the global pass, when scheduled, with three requirements the
      failures dictate — peer edge segments as *soft* (negotiable) occupancy,
      not hard bands; a reserved approach right-of-way for every node port (a
      port must stay reachable — both failures are unreachable endpoints);
      and rip-up-and-reroute or simultaneous assignment so route quality stops
      depending on declaration order. **Scheduled and implemented as M16
      below**; until it landed, the post-M13 **sealed-port fallback** (see the
      hardening section above) addressed the two named failures locally —
      when no route is found it slides a lone side's port along its side
      until an escape pocket opens, restoring "never through a node" on
      `examples/sides.dgmr` (swept with no carve-out) — but it was a port-side
      mitigation, not the arbitration this item calls for: the quality and
      order-dependence points above stood until the global pass landed.

**Blocked by:** M11. **Blocks:** M13 (they build on the final geometry so the
cosmetic work is done once).

## ✅ M13 — Label placement engine + parallel-edge spacing

Edge labels used to be placed blind at the midpoint of their polyline's
longest segment (M9's known limitation); parallel edges sit close enough that
a label's white knockout covers a neighboring edge. Resolved: the anchor is
now computed against the finished geometry, and the fan/lane separation is
label-aware.

- [x] **Move label anchors into layout** (decided): `assemble` computes each
      label's anchor — it knows all geometry: obstacles, lanes, frames, other
      edges — and stores it on `EdgePath` (`label_at: Option<(f32, f32)>`, a
      per-edge field, so the index-correspondence invariant is safe); the
      renderer is slimmed to draw the knockout rect + text at the given anchor
      (`render_edge_labels` keeps the old longest-segment midpoint only as a
      fallback for hand-built paths). `EDGE_LABEL_SIZE` / `LABEL_PAD` moved
      into `layout.rs` (the shared-constant convention) so the knockout rect
      the engine scores is byte-identical to the one the renderer draws; the
      canvas-growth pass now includes label rects (a graze label deliberately
      overhangs its line) and translates anchors with everything else.
- [x] Placement algorithm: candidate positions along each polyline (offsets
      along segments), scored against other edges' segments, other labels'
      rects, node rects, and frame borders/titles; deterministic greedy pass
      with a refinement pass (no randomness, fixed iteration order).
      Implemented as `place_labels` (new M13 section of `layout.rs`). The
      candidates per segment are the five taps in `LABEL_TAPS` (clamped so the
      knockout fits inside a segment long enough to hold it), each in up to
      three variants: **on-line** (the classic knockout-breaks-the-line look)
      and the two perpendicular **grazes** — the knockout's edge still crosses
      its own line by `LABEL_ON_LINE` (2.5 px) so the break reads, while its
      bulk clears one side. The grazes are what resolves genuinely crowded
      gaps: sliding alone cannot dodge a neighbor that runs parallel the whole
      length of a label, and a wide knockout in a tight fan has nowhere to
      hide on-line. Scoring: peer edge lines hidden under the knockout (heavy
      — the defect being eliminated; a label may cover only its *own* line),
      label-rect / node / frame-border / title overlap (flat + area terms,
      nodes heaviest), and small preference terms (segment midpoint,
      on-line over graze) that only break ties. Greedy pass in declaration
      order, then one refinement pass in the same order where each label
      re-optimizes against every other label's current rect and moves only on
      a strict improvement (a later label that crowded an earlier one can be
      relieved by moving the earlier one to its next-best spot). Degenerate
      polylines fall back to the old longest-segment midpoint.
- [x] **Label-aware parallel spacing**: lane/fan assignment informed by label
      sizes so a labeled edge's knockout cannot cover a neighbor; bump
      `FAN_SEP` / `LANE_INSET` / `LABEL_PAD` as needed. Implemented as
      `label_pair_sep`: the gap between two adjacent fan ports (`separate_ports`)
      or jog lanes (`assign_lanes`) is `max(FAN_SEP, wider knockout's cross-run
      half-extent + LABEL_CLEAR)` — the covering extent is read per geometry
      (`side_label_cover`: the knockout's width across a Top/Bottom side's
      vertical runs, height across a Left/Right side's horizontal ones;
      `lane_label_cover`: the flow-axis extent across the lane's cross-axis
      run). Fan bumps are capped at `FAN_LABEL_CAP` (24 px) so a fan of wide
      labels still fits a real node side; lanes need no cap (the gap band
      clamps them) and take their minimums first, sharing the band's slack
      evenly on top — which reproduces the old uniform spread exactly for
      unlabeled groups. When even the minimums overflow the room, gaps shrink
      proportionally (deterministic best effort — the graze candidates resolve
      the rest). One latent bug surfaced and was fixed: `separate_ports`'
      push-and-clamp would collapse several ports onto one coordinate when the
      minimums overflowed the side (collinear stacked legs — an M9-invariant
      violation); it now shrinks the minimum run proportionally instead.
      `LABEL_PAD` stays 3 px (bumping it would grow every knockout for
      marginal stroke clearance; the `LABEL_HIT_PAD` scoring margin covers
      that instead) and `LANE_INSET` keeps its 5 px (lanes hug neither the
      node edge nor the frame border).

Tests: `assert_labels_resolved` on the four samples (exactly the labeled
edges carry anchors; each knockout touches its own line while covering no
other edge's line, overlapping no other label, covering no node interior, and
crossing no frame border or title), placement determinism, the kept
midpoint policy on an unobstructed edge, the degenerate fallback, a synthetic
refinement test (a later label crowding an earlier one out of its best spot),
`label_pair_sep` unit tests, label-aware fan/lane spacing tests, and a
renderer test that text + knockout draw at `label_at`. Snapshots regenerated:
label anchors changed everywhere, and finance's `vpn` fan + gap lanes
respread label-aware.

**Blocked by:** M11.5 (final geometry + unified obstacle model). **Resolves**
M9's known limitation (e.g. finance's `vpn --> cvm1` two-line port list no
longer covers `vpn --> cvm3`'s run or neighboring labels, and a label that
used to sit half-behind a node — `tech.finance.example.com\n443` over
`pubclb` — now moves clear). **Blocks:** M8 only.

---

## ✅ Post-M13 hardening — nested title detours, title-fit frames, non-crossing fan lanes, sealed-port fallback

Plus a **M13.1 obstacle-set flip** (recorded here; see its own entry below):
label knockouts now dodge rendered **arrowheads** and deliberately pay no
attention to **group frame borders** (a label may cross a group's outline;
only the title text stays clear).

Three defects found on a nested `cluster.dgmr` shape; all fixed in
`src/layout.rs` with regression tests. Plus a later routing hardening item
(the sealed-port fallback) that closes the M11.5 known-broken case.

- [x] **Nested title detours.** The M5.5/M10 detour only fired for
      single-frame chains (`chain.len() == 1`), so a stub entering a titled
      frame nested inside another frame ran straight through the inner
      title (`user --> lb` vs. "Application"). `title_detours` now builds a
      per-chain-frame detour list (innermost first): each titled frame is
      crossed beside its title with a jog below it, a clear/untitled frame
      carries the descent coordinate straight through (an untitled frame
      between two detoured ones costs no jog), and the rep port sits at the
      outermost crossing coordinate. `cross_boundary_reach` grew to reserve
      the jog band ([`CROSS_FRAME_PAD`]) for *titled* frames anywhere in a
      top-down cross-boundary chain (untitled deeper frames never host a
      jog; horizontal stubs never detour).
- [x] **Titled frames fit their titles in any viewer.** The frame-width floor
      was `title_w + 2·FRAME_TITLE_X` — 10 px per side, no font-fallback
      allowance — so a title spilled its frame whenever the viewer resolved
      a face wider than the measured one ("Kubernetes Node Group"). The
      floor is now `FRAME_TITLE_X + title_w + title_clear_gap(title_w)` —
      the same fallback-safe margin the detour entries use — for every
      titled frame, any title width.
- [x] **Non-crossing fan lane order.** Lanes were assigned in source-port
      order, which forces crossings when a target's entry row falls between
      the source fan's rows (a target nearly level with the source): one
      sibling's descent crossed the next sibling's source run and then ran
      parallel 2 px from it (`lb --> n2` on `lb --> n1`). `lane_order` now
      topologically sorts the lane group by the non-crossing constraint
      (for Z-routes, `a` before `b` is illegal when `b`'s source cross falls
      in `V_a` or `a`'s target cross falls in `V_b`; a right-to-left gap
      mirrors), deterministic with a base-order tie-break. The enforced
      parallel minimum (`SEGMENT_OBSTACLE_PAD`) also grew 5 → 8 px — still
      below `FAN_SEP` so fan lanes clear — keeping the invariant soft
      (crossings legal, terminal fallback kept: "where achievable").
- [x] **Sealed-port fallback — a lone side's port may slide.** The M11.5
      verdict below records the ladder's sealed-port failure: a peer
      segment's padded band within `SEGMENT_OBSTACLE_PAD` of a port escape
      seals the pocket, `track_route` cannot leave the port at all, and the
      kept-unconditional terminal Z is drawn even when it cuts through a
      node (the `api -- from="top" --> db` and
      `bus -- from="right" to="left" --> queue` shapes of
      `examples/sides.dgmr`). Now, when *no* candidate route clears the
      obstacle world, `route_edge`'s dispatch falls back to
      `port_fallback_route`: the edge retries with its free ports slid
      along their sides — the `PORT_INSET` corner ports (the same
      usable-span ends the fan draws between, i.e. the "7 px from the
      corner" positions), the partner-facing one first, one end moved
      before both — and keeps the first alternative that routes clear.
      A free port is a *lone* side's (a shared side is
      [`separate_ports`](../src/layout.rs)'s to place), never a self-loop's
      or an around-target entry's; a forced side (M11) keeps its side
      literally — only the position along it moves. The fallback only ever
      replaces a defective route with a clear one, never trades one defect
      for another, and is deterministic (fixed try order). This is the
      cheap local version of the global pass's "reserved approach
      right-of-way for every node port" requirement: it removes the
      known-broken pass-through (the `example_edges_never_pass_through_nodes`
      sweep now covers all five samples with no carve-out) but not the
      resource conflict itself — the global pass still owns route quality
      and order-independence.
- [x] **M13.1 — label obstacle-set flip: dodge arrowheads, ignore group
      borders.** Two corrections to M13's scoring, from real output:
      knockouts were kept off the (often large) frame *border bands* while
      happily sitting on a rendered *arrowhead* (e.g. a label whose longest
      segment is the final one into its target). Now:
      - The `LabelWorld` no longer contains `border_bands` — a label may
        cross a group's outline freely (only the frame **title** rect stays
        an obstacle). `assert_labels_resolved` drops the border check
        accordingly.
      - Every routed polyline contributes an arrowhead obstacle:
        [`arrowhead_rect`](../src/layout.rs) — the marker triangle's
        bounding rect (tip on the last waypoint, base `ARROW_LEN` = 10 px —
        the renderer's `markerHeight` — back along the final segment,
        `ARROW_HALF_W` = 5 px perpendicular each side), padded by
        `ARROW_PAD` = 1 px — scored with its own flat + area terms
        (`ARROW_OVERLAP_FLAT` = 80, `ARROW_OVERLAP_W` = 1.2: heavier than a
        title, approaching a node — a hidden arrowhead reads as a broken
        edge). The arrowhead includes the label's *own* edge: a knockout on
        the final segment backs off along the run instead of hiding the
        target arrow.
      - Tests: `label_clears_the_arrowhead_of_the_final_segment` (the bend
        before the target pushes the knockout off the marker),
        `arrowhead_rect_bounds_the_rendered_marker`, and
        `assert_labels_resolved` now asserts no knockout covers any edge's
        arrowhead. Snapshots regenerated (anchors moved only where they had
        covered arrowheads or borders).

---

## ✅ M14 — Forced sides apply to the endpoint node only

M11's literal semantics were implemented at *every* port-decision point: the
forced side was also applied to the group frames the edge crosses, so
`to="right"` on a cross-boundary edge dragged the LCA-level entry to the
outer frame's **right** border — even when the source sat above the group.
The visible result (the `cluster` shape): an excursion around the outer
group's right border, then a cut back across the *other* branch's frames and
under its nodes on the way to the node. The intended meaning is narrower:
the attribute names the side of the endpoint **node**; the group boundaries
between the endpoints keep the layout's natural crossings.

- [x] Each end's sides are split in [`EdgeCtx`](../src/layout.rs): the node
      side (forced or implicit) and the frame side (always [`sides_along`]'s
      implicit one). Rep ports, chain crossings, title detours and the LCA
      segment use the frame sides; only the node ports use the forced side.
- [x] A **mismatched** end (forced side ≠ implicit side) gets a structured
      stub ([`mismatched_stub`]): the rep crossing is clamped into the clear
      corridor beside the node ([`mismatched_rep_cross`] — clear of the
      node's siblings at every chain level and the chain frames' title
      texts, on the side the other endpoint's port leans to), the descent
      runs down it, and the final leg hooks into the forced-side port along
      its outward line. Flow-parallel contradictory sides (`to="bottom"`
      under top-down, facing away from the rep) wrap around the node the
      same way (run below/above its far edge). Every group crossing is still
      an explicit connection point (`line_rect_exit` clipping).
- [x] **Routing must always succeed**: the old coupled reading (the forced
      side applied to the frames too) remains as the terminal fallback
      candidate in [`route_edge`]'s ladder, and an end whose corridor is
      blocked demotes to it. Contradictory sides may still be ugly (M11's
      contract) — but they no longer drag *other groups* into the ugliness.
- [x] Direct edges are untouched ([`forced_direct_path`]); an edge with no
      mismatched end routes byte-identically to M13 — all eleven pre-M14
      snapshots pass unchanged.
- [x] Tests: the cluster regression (corridor descent, no outer-border
      excursion, no sibling-group crossing, both connection points
      recorded), the single-group right-side entry, the contradictory
      bottom-side wrap; `examples/cluster.dgmr` is the canonical sample
      with a golden snapshot.

**Blocked by:** M11/M11.5. **Blocks:** nothing.

---

## ✅ Post-M14 fix — edge-chain endpoints in a group body place their nodes

Referencing a node in a group body is specified (`grammar.md`) to place it in
that group, but `src/resolve.rs` only honored standalone declarations: the
endpoints of an edge chain written inside a `group` body were treated as pure
references, so pre-declared nodes stayed top-level and the group rendered
empty (`examples/sides.dgmr`: `bus --> queue` in "Messaging"). Chain
endpoints in a group body now place their nodes like standalone declarations,
except that an endpoint already placed in a *different* group keeps its group
(a cross-boundary reference — the M5 within-frame shape). The M1 invariant is
symmetric now: top-level occurrences never move a node in either direction, so
a declaration after a group body no longer un-groups its members and
membership is order-independent. Resolve tests cover both chain shapes, the
cross-boundary reference, and nesting; `snapshots/sides.svg` regenerated (the
only golden file affected).

---

## ✅ M15 — Component packing: direction is honored with no edges

**The defect.** Longest-path ranking only constrains nodes that participate
in an edge; edge-less nodes keep rank 0, so every edge-less node in a scope
crowded into the source band — a single row (top-down) or column (left-right)
running **perpendicular** to the declared flow axis. A top-down diagram's
unconnected nodes laid out left-to-right; a left-right group's unconnected
members stacked vertically. The declared direction was honored only where an
edge existed to enforce it.

- [x] **Component packing** (`pack_components`, run after `longest_path` and
      before dummy insertion): union-find the weakly connected components of
      the level's DAG (self-loops excluded — they don't constrain rank) and
      re-base each component onto a disjoint run of ranks, stacked along the
      rank axis in declaration order of each component's first member,
      separated by one inter-rank gap. Within a component the relative ranks
      (and therefore the internal arrangement) are preserved (a uniform
      shift); a fully connected scope is unchanged byte-for-byte. Because it
      re-bases *ranks* rather than post-translating y, every downstream phase
      (dummies, crossing minimization, coordinate assignment, band stacking)
      sees consistent global ranks for free, at every level of the compound
      recursion.
- [x] **Semantics (decided):** `group <direction>` (and the diagram header)
      now names the internal shape *unconditionally*: `left-right` gives a
      side-by-side row, `top-down` a vertical stack, edge or no edge. The
      perpendicular band was never a feature — it was the ranking's
      unconstrained fallback.
- [x] Samples: `infra`'s "Kubernetes Cluster" and `finance`'s "Private
      subnets" declare `left-right` (their side-by-side member rows are the
      design), and `cluster`'s "Application" declares `top-down` (its
      vertically-stacked security groups relied on the old fallback); the
      `group_style` snapshot's inline test source likewise declares
      `left-right` on its styled group (edge-less members C/D sat side by
      side only via the old fallback). Snapshots `isolated_nodes`, `sides`,
      `group_style`, `finance`, `cluster`, `infra` regenerated.
- [x] Tests: edge-less nodes stack along the flow axis top-down and run
      left-to-right in left-right; an inherited-direction group's edge-less
      members follow the inherited axis.
- [x] **Known limitation (recorded, deliberately not patched here):** a
      frame whose members are stacked along its flow axis *and* receive
      cross-boundary edges is a shape the per-case routing regime does not
      satisfy — a within-frame stub can cut a stacked sibling, and routes
      from different stacked sources sharing a frame side can coincide (the
      regime separates fan lanes per source, not across sources). These
      shapes were rare pre-M15 (only connected deep chains produced them,
      partly handled by `try_around_target_route`); M15 makes them
      user-reachable with plain edge-less members. Per the M11.5 verdict
      below, the cure is the **global routing pass** (peer segments as soft
      occupancy, reserved port right-of-way, rip-up) — implemented as **M16**
      below, with the stacked shape as a regression test
      (`stacked_group_cross_traffic_keeps_ports_reachable`).

**Blocked by:** nothing (ranking-level change). **Blocks:** nothing, but it
broadens exposure to the known routing limitation above.

## ✅ M16 — Global routing pass (the M11.5 verdict, implemented)

The M11.5 verdict recorded that the one-pass regime satisfies the invariants
only through per-case fallbacks, because it cannot arbitrate a resource
conflict: an already-routed peer segment's hard band can seal a later edge's
port escape pocket, and whose route yields then depends on declaration order.
Its three requirements are implemented here as a two-phase routing regime in
`assemble`, on top of the unified M11.5 machinery (one obstacle world, one
ladder, one dispatch point — all kept).

- [x] **Soft peer occupancy.** `Obstacles` splits peer segments into a hard
      list (empty in the global pass; kept for the unit fixtures and as an
      absolute backstop for callers that want it) and a soft list
      (`SOFT_RIDE_COST` = 100 per parallel-riding band). Nothing is barred by
      soft occupancy: `route_lca`'s ladder and `route_edge`'s dispatch accept
      the least-conflicting hard-clear candidate (first zero-conflict
      candidate wins outright; earliest wins ties), and `track_route` charges
      soft bands in its Dijkstra cost so routes prefer clear corridors but
      may cross negotiated space. A conflict is resolved by rip-up, never by
      forcing the current edge into a worse shape to dodge an earlier edge's
      parked band.
- [x] **Reserved port right-of-way.** Every routed edge reserves the pocket
      each of its ports escapes through — `pocket_rects`: the swept rect of
      the *actual* terminal approach segment (first/last, truncated to
      `PORT_RIGHT_OF_WAY` = `SIDE_ESCAPE`, grown by `PORT_CORRIDOR_PAD` = 3).
      Derived from the route itself rather than the nominal side, so an edge
      whose real entry is perpendicular (an around-target route, M14's
      decoupled bends) reserves the corridor it uses and never blocks a
      sibling's straight approach through its unused nominal port. Pockets
      are hard for peers (parallel riding inside barred, perpendicular
      crossing legal — the `seg_clear_padded` polarity), invisible to the
      edge's own route, and re-derived from the live polylines each repair
      pass so a moved approach also moves its reservation.
- [x] **Simultaneous pass + rip-up-and-reroute.** Phase 1 routes every edge
      against the base world only (nodes + LCA-level frame borders) — the
      simultaneous assignment: every edge gets its unconstrained-best route,
      independent of declaration order (verified: phase-1 polylines are
      identical under edge permutations — enshrined as a test in M17). Phase 2 is a bounded
      (`MAX_RIPUP_PASSES` = 8) repair loop: `scan_violations` finds (a)
      hard-world defects (a segment through a node or riding a border — the
      kept-unconditional terminal) and (b) collinear peer riding within
      `SEGMENT_OBSTACLE_PAD`, attributing each ride by the pockets — a rider
      inside the *other* edge's pocket yields alone (right-of-way), a rider
      inside its *own* pocket stays while the other yields, a mid-route ride
      away from any pocket is mutual; the ripped edges reroute in index
      order against the live paths (a later ripped edge sees an earlier
      one's already-moved segments, so two edges that parked on the same slot
      cannot both keep it), and an edge still violating after its pass is
      frozen (its violation is the bounded best effort, like the ladder's
      terminal).
- [x] **Gap-wide lane coordination (M9 refinement, absorbed here).** M9's
      lane separation grouped jogs by (source, side, gap), which left
      cross-source sharers of one gap uncoordinated — two edges from
      different sources could park their jogs a few px apart and only the
      declaration-order hard bands kept them apart. `assign_lanes` now
      groups by (side, gap) alone and shrinks the band away from the union
      of the members' source-side obstacles, so every jog in one band is
      spread together — the simultaneous-assignment reading of lane
      separation. Per-source separation still holds (a superset); the M9/M13
      lane tests pass unchanged.
- [x] **The two recorded failures now route by negotiation.** On
      `examples/sides.dgmr`, `api -- from="top" --> db` and
      `bus -- from="right" to="left" --> queue` — the sealed-pocket edges —
      route clear without the port-fallback carve-out, and the former is
      simpler than its pre-M16 shape (escape top, across, down into db — no
      ladder fallback needed); the sealed-port fallback stays as the
      backstop for ports that still cannot escape.
- [x] **Frame rep-crossing fan (finishing pass).** Cross-boundary edges whose
      chains are non-empty converge on their representative's frame crossing
      at the member's own cross (node-aligned entry); when several edges hit
      the same member — or members placed level — the crossing coordinates
      are fanned apart on the frame side, minimum [`FAN_SEP`] (like node
      sides, [`separate_ports`] with `frame: true`). Mismatched ends keep
      their M14 corridor clamp (no fan entry); an around-target edge's
      crossing is unused by its own generator but still fanned, so its
      fallback path gets a crossing of its own — its node ports are fanned
      on the implicit side for the same reason (the around-router reads node
      geometry directly, so neither move disturbs the around-route).
- [x] **Negotiation vocabulary widened (finishing pass).** The repair loop
      needs candidate generators that can actually *step aside*; four gaps
      the dense-graph stress sweep exposed are closed:
      `track_route` seeds tracks at a band's centerline, so a route crosses
      a frame's drawn outline legally in two perpendicular hops (riding
      along it stays barred — the crossing waypoints survive collapse via
      `record_border_crossings`); the within-frame stub's sibling-avoidance
      detour ([`force_stub_around_siblings`]) now applies to unforced ends
      too (port separation can clamp a port cross onto a sibling's column)
      and detours inside the chain frame's border bands with the routing
      world's peer occupancy as soft cost, so two edges' detours around one
      sibling negotiate into distinct corridors; the around-target
      generator jogs its initial descent around whatever blocks the source
      column (track-routed), records crossings against the whole world's
      borders (not just its own frames), and fans its side-run outward in
      [`FAN_SEP`] steps when the nearest corridor is occupied; and
      `assign_lanes` groups edges by gap-span *overlap* per side (not by
      the exact span), so a wide-gap edge and a narrow-gap edge sharing one
      band spread their jog rows together — the group's band is the
      intersection of its members' spans, shrunk away from the band
      obstacles.
- [x] Tests: the stacked-group M15-limitation regression (no pass-through,
      no overlap, every edge on its target's boundary — a reachable port);
      cross-source gap-lane coordination (≥ [`FAN_SEP`] apart); a unit sweep
      of `scan_violations`' attribution (clean / hard defect / mutual ride /
      pocket invader); and a dense deterministic compound stress graph (24
      nested-group nodes, ~17 edges from a fixed LCG) sweeping all the
      invariants at once. `example_edges_never_pass_through_nodes` and the
      M9/M11 invariant suites sweep all six samples with no carve-out.
      Snapshots regenerated where negotiation improved a route: `finance`
      (the cross-source fan lanes respread; the big port-list label now has
      a clean anchor), `group_style` (two cross-boundary lanes spread to the
      band's edges instead of coinciding at the midpoint), `sides` (the two
      forced edges renegotiated). All other snapshots byte-identical.
- [x] **Bounded residue (documented).** When the repair cannot fix an edge
      (no candidate clears the world and no port alternative opens), the
      edge freezes with its least-bad route — the same bounded best-effort
      the ladder's kept-unconditional terminal always represented. Phase 2's
      conflict splitting is index-ordered (deterministic); it only fires
      among directly conflicting edges, which the gap-wide lane grouping and
      the port fans make rare.

**Blocked by:** M11.5 (the unified regime it refines). **Blocks:** nothing
new; M8 remains the only open milestone.

## ✅ M17 — Post-M16 review fixes (lane union-merge, permutation proof, #cleanup)

Three findings from the M16 review, all minor — no behavior regression, one
snapshot regenerated:

- [x] **`assign_lanes` groups by union interval merge.** M16 tested group
      membership against the *head* edge's span only, which (a) split
      chained overlaps (A[0,10], B[5,15], C[12,20] left C unmerged despite
      overlapping B) and (b) admitted groups whose members pairwise overlap
      the head but not each other — an empty intersection, under which every
      member then got the lane `gap_lo`, possibly *outside* its own gap.
      Membership is now tested against the running union `(min lo, max hi)`,
      so chains merge. When the merged members' intersection is empty there
      is no common band: each lane is clamped into its own edge's gap — it
      starts at its own gap's midpoint and is fanned apart left-to-right,
      then restored right-to-left where a clamp bunched an end (the same
      three passes [`separate_ports`] fans with), so identical or nested gaps
      still get distinct jogs instead of parking collinearly on one
      midpoint. The dense stress graph exercises the empty-intersection
      branch; only `snapshots/sides.svg` changed (one cross-boundary jog
      took its union-merged fan slot), all other snapshots byte-identical.
- [x] **Phase-1 permutation invariance enshrined in a test.**
      `phase1_polylines_do_not_depend_on_edge_order` lays out the same
      diagram twice — edge lines reversed — and asserts node rects and
      phase-1 polylines identical per id. Making the claim true in general
      surfaced two latent tie-break leaks: both the lane sweep's sort and
      `separate_ports`' stable sort broke exact-tie ties by edge *index*,
      i.e. by declaration order; they now break on the edge's endpoint ids
      (`LaneEdge`/`PortEnd` carry `from_id`/`to_id`). Note the *final*
      (post-phase-2) routes still differ under permutation on
      `sides`/`cluster`-shaped conflicts — the documented bounded residue:
      phase 2 negotiates in index order by design, so the headline only
      holds for the simultaneous phase.
- [x] **Debug scaffolding stripped.** The `DIAG_DBG` eprintln blocks in
      `assemble` and `scan_violations` are gone. The `#[ignore]`d dump test
      keeps `DIAG_SRC` (documented in AGENTS.md) as inspection tooling.

**Blocked by:** M16. **Blocks:** nothing; M8 remains the only open
milestone.

### ✅ Post-M17 hardening — no route doubles back over its own escape stub

Reported on `examples/sides.dgmr`: the `api --> cache` edge (forced
`from="left" to="right"`) escaped API's left port, then rode *back* along its
own escape stub's line to a free track before descending — an out-and-back
spike the obstacle world can never see (it contains nodes, peers, corridors
and borders, but never the edge's own stub). Two defects of one class:

- [x] **Escape-stub bars in the ladder.** The router now carries the
      outward direction of each port stub attached around a routed span
      (`Escapes`, threaded through [`route_lca`]/[`track_route`], populated
      by `forced_direct_path`, `lca_segment` and
      `force_stub_around_siblings`). A Z rung whose first leg runs toward
      the start port or whose final leg runs outward into the end escape
      point is not a candidate; the Dijkstra bars the same two leg shapes,
      so it finds the best *constrained* route (here: descending straight
      at the escape point) instead of the spiked one.
- [x] **Around-target lane spike.** The stricter check then exposed a
      second, pre-existing spike: in the around-target builder's `outside`
      shape, a descent ending between the frame border and the around-lane
      rode out to the lane and reversed back through the crossing
      (exercised by `stacked_group_cross_traffic_keeps_ports_reachable`'s
      `s2 --> m2`). The lane waypoint is now skipped when the descent
      already ends on the target's side of it — it crosses into the frame
      directly.
- [x] **Invariant enshrined.**
      `assert_no_edge_overlaps_or_node_passage` also asserts no edge
      overlaps *itself* (two of its own segments sharing a collinear
      overlapping span — a spike; perpendicular corners and re-inserted
      border-crossing waypoints don't false-positive).

`examples/cluster.dgmr` and every other example render byte-identically;
only `snapshots/sides.svg` changed (the fixed `api --> cache` route plus its
downstream negotiation with the `api --> db` forced-top edge, which moved
its descent corridor to stay clear of the new lane).

## ✅ M18 — Multi-frame within-frame stubs avoid siblings (option 1)

Diagnosed on `dev-digital.dgmr` (kept out of the snapshot suite and
permanent tests per its owner): the edges

```
node_a -- dotted "8091 gRPC\n8090 HTTP" --> polaris
node_a -- dashed "event logs" from="right" --> cos_logs
```

leave node_a's right side and run **straight through node_b** — collinearly
on top of node_b's own `--> polaris` / `--> cos_logs` first legs. Root
cause (established by instrumenting `route_edge` with temporary probes,
since removed; the committed SVG reproduced the defect byte-identically):

```
diagram left-right
group
group
A
B
end
end
C
A-->C
B-->C
```

- Both edges' from-chain is **two frames deep** (`[Worker Node Pool, TKE
  Cluster]`), and their endpoint siblings sit side by side with `node_b`
  spanning the *entire* useful range of node_a's right side — no
  right-side port choice avoids it; only a vertical jog can.
- [`stub_for`] gated the obstacle-aware, Dijkstra-routed sibling-avoidance
  builder ([`force_stub_around_siblings`], which runs [`track_route`] with
  the innermost frame's siblings as solid obstacles, its border bands, soft
  peer occupancy, and escape bars) on `chain.len() == 1`. With a two-frame
  chain it fell through to [`build_stub`] — a purely geometric
  node-aligned line clipped at frame boundaries, with **no obstacle
  awareness** (deliberate per M9: "within-frame stubs keep the M5/M7 logic
  unchanged"; M11.5's one-regime hoist never covered the stub builders).
- `route_edge` therefore emitted exactly **one** candidate whose first
  stub leg fails `world.clear()` against the node_b rect; per the
  bounded-ladder convention the terminal candidate was kept
  unconditionally. M16 phase 2 re-detected the hard violation every pass,
  got the same single failing candidate, and **froze** the edge (the
  sealed-port fallback could not move the port either — the side is
  shared by both edges).
- So: **not** a Dijkstra node-coverage problem — the router was never
  invoked for the broken span. The LCA segments, lanes, and label anchors
  were all clean; only the blind multi-frame stub leg was defective.

The milestone (option 1 of the two considered; option 2 is M19) generalized
the existing sibling avoidance to chains of any length. Checklist:

- [x] **Generalize the `chain.len() == 1` gate** in [`stub_for`] so
      [`force_stub_around_siblings`] handles multi-frame chains. The
      obstacle set grows to include **every chain frame's border bands**
      (not just the innermost), so the frame crossings the straight stub
      clipped legally stay legal *designated crossings* for the detour —
      the shared world only ever contained the LCA-level frames' bands, so
      without this the detour could graze a nested outline unchecked.
- [x] **Fix the Dijkstra start point.** `force_stub_around_siblings`
      seeds [`track_route`] via a clamped escape instead of the raw
      [`escape_point`] walk (24 px out along the side's normal, which for
      a side-by-side sibling lands *inside* the sibling's clearance band).
- [x] **Keep the escape bars and soft occupancy** (the M17 escape-stub
      spike hardening) active on the generalized path.
- [x] **Regression guards.** The existing single-frame tests
      (`cross_boundary_stub_does_not_cross_sibling`,
      `sibling_fan_edges_never_cross_or_ride`, the forced-side chain
      tests) stayed green and shape-identical. New focused test
      `multiframe_stub_avoids_side_by_side_deep_sibling` — the synthetic
      two-deep side-by-side fixture above (the `dev-digital.dgmr` diagram
      itself stays out of the permanent suite and snapshots): A→C must not
      pierce sibling B, must run orthogonal to C's boundary, and must not
      ride B→C.
- [x] **Snapshot triage:** all committed examples render byte-identically
      except three regenerated goldens (see below); `dev-digital.dgmr`
      byte-identical throughout.

Implemented in `src/layout.rs`:

- **Gate.** `stub_for` now hands *any* chain length to
  [`force_stub_around_siblings`]; title detours still short-circuit ahead
  of it, and the trigger stays narrow — a multi-frame stub detours only
  when the straight stub fails the clearance check across the whole
  chain-frame world, so prior geometry is kept whenever the straight stub
  is legal.
- **Chain-frame bands + crossings.** The detour's obstacle world carries
  every chain frame's border bands (not only the innermost frame's), and
  `record_chain_crossings` re-records the routed legs' frame transversals
  as explicit waypoints — the same designated-crossing convention the
  straight stub's `line_rect_exit` clipping satisfied.
- **Clamped escape (rewritten).** `clamped_escape` walks the escape ray
  from the port and cuts just short of the first *orientation-matched*
  barrier: a node the [`ROUTE_PAD`]-inflated clearance check would
  pierce, a reserved port pocket or hard peer band the ray would ride
  parallel inside, or a border band it would cross. Two defects this
  repair surfaced in the earlier prototype are fixed here: the
  "nearest barrier" comparison compared a candidate's own distance
  against itself (the first obstacle along the ray always won instead of
  the nearest — the seed was born two rows deep inside a far sibling and
  the uncancellable straight port→seed leg then pierced everything in
  between), and reserved pockets/border bands were absent from the
  barrier set (the old escape walked straight into a sibling's approach
  pocket). With both fixed, the seed is born in free space, and the
  sibling tracks (± [`TRACK_OFF`]) supply the jog around it.
- **Bounded negotiation retry.** Stub-detour and ladder track routes get
  one retry (`track_route_negotiated`): when the routed polyline rides a
  parked peer band *exactly* (same centerline, overlapping span), the
  ride is pinned as a border band and the Dijkstra re-runs — a clear route
  that is strictly less soft-conflicting moves; otherwise the ridden
  route stands (the bounded-ladder convention). Phase 1 has no peers, so
  the retry only ever fires in the phase-2 repair loop; the phase-1
  permutation invariance test is untouched.
- `dev-digital.dgmr`'s two offending edges now route with an obstacle-
  aware detour on their multi-frame stubs; the committed `dev-digital.svg`
  is byte-identical to the pre-M18 drawing (the defect had been fixed in
  the working tree before this doc pass; the invariant is pinned by the
  new test).

Snapshots: `cluster`, `finance`, and `sides` were stale — their pinned
geometry shifted with the earlier source-rank centering revision and had
not been regenerated; they are regenerated and hand-reviewed here (only
benign 1–3 px fan/row respreads and label anchors riding rows that had
already moved; no defect-class routes). All other snapshots are
byte-identical.

**Known residue (handed to M19).** The dense-stress invariant sweep
(`global_pass_stress_dense_compound_graph`) still fails on one pair: two
cross-boundary edges whose target stubs must traverse a single-row
corridor above the Inner group (one track row sits between the frame's
top band and the inner members' clearance padding) park their spines on
that row and ride collinearly for ~13 px. Each edge alone routes clean
(phase 1 accepts both); the defect is only visible pairwise, so no single
edge's reroute can see and clear it, and the negotiated retry finds no
alternative because the corridor physically offers one seeded row under
the current track seeding. The trunk-over-trunk ride the sweep reported
earlier (n18→n19 across n16→n22's escape) **is** fixed by the
clamped-escape repair; the sweep's remaining failure is exactly this
residual. Closing it needs row-fan coordination across simultaneously
routed detours sharing a corridor — plus corridor seeding that offers
interior rows beyond obstacle-edge ± [`TRACK_OFF`] when only one exists
— both of which belong to the M19 unified dispatch.

**Blocked by:** nothing (M17's escape bars are a constraint the fix must
respect, not a blocker). **Blocks:** M19.

## ✅ M19 — Stub routing unified into the one-regime dispatch (option 2)

The end-state refactor M18 deliberately defers: complete the M11.5 unifying
move — "one obstacle world, one routing regime" — over the *stub* spans,
dissolving the sibling special case instead of extending it. M19 also owns
the M18 residue: row-fan coordination across simultaneously routed detours
that share a corridor row (see M18's "Known residue"), plus corridor
seeding that offers interior rows between a frame band and member padding
when obstacle-edge ± [`TRACK_OFF`] seeds supply only one — enough vocabulary
to clear the dense-stress sweep completely.

**This milestone is planned as three sessions** (S19a–S19c below). The
refactor-with-invariance-contract work (sessions a and b) is kept separate
from the new-geometry residue work (session c): the former expects
byte-identical output, the latter expects output to change — so each
session's snapshot triage stays unambiguous. Sessions must run in order: b
absorbs a's machinery, and c's coordination mechanism must operate on the
final dispatch shape so it does not have to be re-proven after the
refactor.

### ✅ S19a — Dispatch unify, minimal surface (refactor; expect byte-identical)

- [x] **Route any failing stub span at the dispatch.** When a candidate's
      straight [`build_stub`] output fails `world.clear()`, reroute the
      port→rep-port span through [`track_route`] against the shared world —
      the same treatment the LCA segment already gets — rather than
      emitting the defective straight stub as the unconditional terminal.
      The straight stub remains the kept-unconditional fallback (the
      bounded-ladder convention) when no clear route exists. *(Landed as
      `reroute_stub_span`, called from `stub_for` right after the
      mismatched early return — M18's geometry generalized to any failing
      straight span: chain bands hard, peer occupancy soft, clamped
      escape seed, `record_chain_crossings` for the detour's own
      transversals; the sibling set under the innermost frame is in the
      world as nodes.)*
- [x] **Chain bands in the world for that span.** The shared world carries
      only the LCA-level frames' bands (see [`edge_world`]); the routing
      adds the chain frames' bands for exactly the stub span
      (`reroute_stub_span`'s stub-local `Obstacles`), so a rerouted leg
      never rides a nested outline unchecked (the designated crossings the
      straight stub clipped are legal; a grazing rerouted leg is not).
- [x] **Session-local constraint:** M18's [`force_stub_around_siblings`]
      and its gate stay untouched; the dispatch reroute runs as a checked
      gate ahead of it. *(The S19a reroute is gated to multi-frame chains
      — `chain.len() > 1` — for determinism: a single-frame chain's
      accepted M18 fallback geometry is pinned by unit expectations, and
      its detour's escape lands inside the S19a would-be obstacle world
      (a sibling 24 px out on the escape normal), so routing against that
      world here could flip which accepted fallback it lands on — a byte
      change this session must not introduce. S19b absorbs the special
      case and retires the gate.)*
- **Deliverable:** suite green; snapshots byte-identical vs post-M18 *(all
  twelve golden files verified by checksum against the post-M18 state)*;
  dense-stress sweep status re-measured against the new failure surface
  *(same single defect pair remains — `n6→n14` vs `n1→n14`, the corridor
  row-fan in S19c's scope; phase-1 probing shows the two full chain-ride
  spans there are already derailed onto distinct corridors, and the
  failure boundary is unchanged at the same segment pair — no new defect
  class surfaced by the reroute gate)*.

### ✅ S19b — Absorb & retire, cover the last blind span (mostly refactor)

- [x] **Absorb M18's machinery, then retire the special case.** The
      generalized [`force_stub_around_siblings`] and its chain-length gate
      become redundant: move the chain-local obstacle construction to the
      dispatch point (or a helper it calls), keep the geometry, delete the
      gate. `cross_pieces_candidate`'s candidate list simplifies rather
      than grows another fallback layer.
- [x] **Cover the other blind span too:** `mismatched_stub` for multi-frame
      chains is purely geometric today (M14); the unified regime should
      give it the same clearance-checked reroute. *(Landed in `stub_for`'s
      mismatched branch: the built mismatched stub is checked against the
      shared world and, when blocked, rerouted through
      `reroute_stub_span`/`stub_world` — the same chain-bands-hard,
      peer-occupancy-soft regime — with the mismatched geometry as the
      kept-unconditional fallback. A single-frame chain keeps its
      M14-pinned geometry unconditionally: its world is exactly the
      sibling/title facts `mismatched_rep_cross` already cleared. Two
      supporting fixes surfaced while proving the branch with a unit
      fixture: (1) `clamped_escape` clamped its seed against raw border
      bands but `seg_clear_border` bars the `BORDER_PAD`-inflated extent,
      so a seed could be born inside the barred zone and a genuinely clear
      detour was falsely declined — the clamp now uses the inflated band;
      (2) `stub_world` now carries the chain frames' title texts as solid
      nodes (measured glyphs + the detour's own `title_clear_gap` margins),
      so a reroute can never trade a title detour for a route that crosses
      the title the detour was built to avoid.)*
- **Deliverable:** suite green. Unlike S19a, byte changes are permitted
  here but only as defect fixes of exactly the blind-stub class (the
  previously blind `mismatched_stub` span may begin avoiding defects it
  never saw); every byte change is hand-reviewed and triaged as such.
  *(All twelve golden snapshots verified byte-identical — no current
  sample exercises the new branch, so there was nothing to triage; the
  branch is pinned by the unit fixture
  `mismatched_multiframe_stub_gets_the_dispatch_reroute`, which reproduces
  the blind defect (a mismatched descent riding another edge's reserved
  port corridor) and asserts the reroute clears it while a single-frame
  chain keeps its geometry.)*
- **Invariants re-verified end to end (a+b together):** phase-1
  permutation invariance, escape-stub bars, pocket right-of-way, and the
  snapshot suite.

### ✅ S19c — M18 residue: corridor coordination (new geometry; output expected to change)

- [x] **Row-fan coordination** across simultaneously routed detours that
      share a corridor row: M18 established that no single-edge reroute
      can see the pairwise defect (two target stubs parking collinear
      spines on the one-row corridor above the Inner group), so the
      coordination must be a simultaneous-routing mechanism. *(Landed as
      two cooperating mechanisms, both proven load-bearing by reverting
      each and watching the sweep fail: (1) the escape seed treats soft
      peer bands as barriers — `clamped_escape` cuts the seed short of
      any soft band the escape ray enters (orientation-agnostic, unlike
      the hard kinds: a hard parallel band bars the escape leg itself,
      while a soft band only poisons a seed born inside it — whose
      approach leg then rides the peer's centerline exactly and no later
      negotiation can clear, because the phase-2 retry pins the ridden
      band as a border and the Dijkstra, starting inside the pinned
      obstacle, finds no route. Seeding short parks the seed in a free
      row and the band's own edge-seeded track rows give the
      simultaneously rerouted detours distinct corridor rows to fan
      onto, in deterministic repair order); (2) the dispatch consults
      the M16 frame rep-crossing fan for *every* cross-boundary edge —
      `route_edge` now always pushes the decoupled `cross_pieces_candidate`
      first (it reads `ctx.{from,to}_rep_cross`, the fanned crossings),
      keeping the coupled reading as the terminal fallback. Before S19c
      the decoupled candidate was pushed only for mismatched ends, and
      the M14 claim that the two readings are "identical for edges with
      no mismatched end" had been silently broken by the M16 fan: an
      unforced edge never consulted it, so two edges into one frame
      parked their LCA approaches on one shared column.)*
- [x] **Corridor seeding that offers interior rows** between a frame band
      and member padding when obstacle-edge ± [`TRACK_OFF`] seeds supply
      only one — enough vocabulary for the coordination to spread onto.
      *(Found unnecessary: with (1)+(2) in place the negotiation's
      band-edge rows suffice — the defect pair fans onto rows 9 px apart
      (a band edge vs the `BORDER_PAD`-inflated frame band's own edge
      track). A third change tried during the session — generalizing
      `lca_segment`'s escape bars to flow-parallel rep ports (seam
      bars: the LCA span may not double back over the within-frame stub
      it meets at the port) — turned out to be masking, not fixing: the
      seam spikes were downstream symptoms of the shared rep column,
      and with the fan consulted the bars stay scoped to the
      perpendicular (escaped) ports as before. The lesson is recorded by
      the `lca_seam_never_reverses_over_the_stub` fixture, which pins
      the seam's no-self-overlap invariant on the deep cross-group fan
      geometry that produced it.)*
- **Deliverable:** the dense-stress invariant sweep
  (`global_pass_stress_dense_compound_graph`) fully green *(the last
  defect pair — n6→n14 vs n1→n14's collinear spines on the one-row
  corridor — now fans onto distinct rows)*; the affected snapshot
  regions hand-reviewed *(all twelve golden files regenerated and
  byte-identical to the committed state — no sample exercises the new
  mechanisms hard enough to move a route; the only byte change seen
  during the session was cluster.svg under the later-reverted seam bars,
  hand-reviewed and then moot)*; phase-1 permutation invariance stays
  pinned *(both mechanisms are simultaneous-assignment-clean: the seed
  clamp reads only the shared world's facts, and the fanned crossings
  come from the pre-pass fan whose tie-breaks are content-keyed)*. Two
  unit fixtures pin the mechanisms:
  `unforced_cross_edges_use_their_fanned_rep_crossings` (two unforced
  edges into one frame side must cross the border on distinct fanned
  columns) and `lca_seam_never_reverses_over_the_stub` (the LCA↔stub
  seam draws no out-and-back spike on a deep cross-group fan).

Rationale for the sequencing (M18 before this milestone): M19 needs M18's
geometry proven (chain bands, the clamped track start, designated
crossings) against [`route_edge`]'s candidate-priority and
terminal-fallback semantics — the most invariant-laden dispatch in
`layout.rs`; doing it in one step means inventing and proving all of it
there at once. M18 shipped the fix with a narrow trigger and maximal reuse
of pinned tests; M19 then *absorbs* it (none of M18's logic is discarded,
though the `chain.len() == 1` code shape is retired) and closes the class
for every stub kind, including the ones M18 does not touch.

**Blocked by:** M18 (proves the geometry it generalizes). **Blocks:**
nothing; M8 remains the last open milestone.

## ☐ M20 — Whole Path Maze Routing

See ./plan-for-whole-path-maze-routing.md

### ✅ M20.1 — Solid/ancestor classifier + track-graph region widening (foundation, no behavior change)

The plan's foundation milestone, landed exactly as scoped: the per-edge
frame world exists and is wired, and the track-graph machinery accepts a
caller-chosen region — but no routing behavior changed. All twelve
golden snapshots are byte-identical to the committed state (verified by
the suite and by re-rendering the examples).

- [x] **The classifier ([`FrameClass::of`])** — a pure function of the
      containment tree and the two endpoint node indices. For edge `e`
      with endpoints `s`, `t`:
      - **ancestor frames** — every frame on the ancestor-or-self chains
        of `s` and `t` (the chains [`chain_to_lca`] reads, unioned and
        index-sorted). Perpendicular crossing legal (a search output
        from M20.2 on), parallel riding on the band barred — the existing
        border-band semantics without the rep-port / stub-crossing
        decomposition.
      - **solid frames** — every other group frame (siblings, unrelated
        deep subtrees). Fully blocked, interior *and* border band:
        `nothing may touch them`; a corridor *between* two solid frames
        is open space and legal. This kills the
        threading-through-an-unrelated-group problem by construction.
      - **`chains_disjoint`** — true when neither endpoint's group is an
        ancestor-or-self of the other's (the genuinely cross-boundary
        shape); false for nesting endpoints and when either endpoint is
        top-level (`None` contains every group, so no group-vs-group
        disjointness applies there).
      The LCA concept, rep crossings, stub/rep decomposition, and the
      M14 coupled/decoupled duality all *survive untouched today* — the
      current dispatch still routes through them. What changes is that
      the containment information is now available per edge in the shape
      the whole-path search consumes.
- [x] **[`EdgeFrameWorld`]** — the classification plus the obstacle
      fields built from it (ancestor bands via [`border_bands`], solid
      raw rects), built once per edge in [`assemble`] and carried on
      [`EdgeCtx`] (with a pure on-demand fallback for hand-built unit
      fixtures). Its three predicates — [`EdgeFrameWorld::clear`] /
      [`seg_clear`]/[`seg_clear`] segment form,
      [`EdgeFrameWorld::cross_cost`], and [`EdgeFrameWorld::track_seeds`]
      — are the whole-path search's (M20.2) scoring and vocabulary:
      [`cross_cost`] charges [`FRAME_CROSS_PENALTY`] \(300, tuned once:
      ~25 bends' worth, so a whole-path search prefers one crossing per
      ancestor frame over a cheap-looking double-cross) per *mid-segment*
      perpendicular transversal of an ancestor band, keeping a segment
      that *ends* on a centerline (a designated crossing) free — exactly
      [`seg_clear_border`]'s exemption; [`track_seeds`] seeds ancestor
      bands at edges ±[`TRACK_OFF`] *and* their centerlines (the
      designated-crossing rows [`track_route`] already uses) and solid
      frames at edges only (nothing may touch them).
- [x] **The track graph accepts a caller-chosen region.** The Dijkstra
      core was extracted into [`track_search`] \(graph + start/goal +
      world; the search itself unchanged, byte-identical behavior);
      [`track_route`] now calls it with its own derived
      ports-and-obstacles region, and the new
      [`track_route_in_region`] calls it with a caller region. The
      region a whole-path search uses is [`around_region`]: the frame
      world's obstacle extents plus every node rect, pushed out by
      [`AROUND_MARGIN`] (60) on every side, clamped to the canvas —
      with the canvas clamp being the load-bearing half (the old derived
      region can extend past the canvas and "detour" through undrawable
      space; the widened one cannot).
      [`whole_path_route`] ties the pieces together — the M20.2 router
      (live from M20.2 on, see below).
- [x] **Tests** (7 new + 2 mechanism tests):
      - classification: root-level endpoints (unrelated frame solid),
        both endpoints in one group, nested chain (both frames
        ancestors), sibling groups (mutual ancestors, disjoint), an
        unrelated nested subtree (solid), deep nesting with a sibling
        wall, and a nesting endpoint pair (shared tail, sibling solid);
      - [`EdgeFrameWorld`]: 4 bands per ancestor frame, the solid rect
        raw, segment-clearance semantics (clear between frames, barred
        parallel on a band, barred through a solid interior), and the
        cross-cost ordering (one straight crossing < an out-and-back, a
        designated crossing free);
      - [`around_region`]: the margin shape and the canvas clamp;
      - [`track_route_in_region`]: a solid wall flush to the canvas top
        — the local derived region "detours" *above the canvas* (the
        defect region widening closes), the widened region finds the
        drawable corridor below the wall and stays inside the canvas;
      - `whole_path_route`: exercised through its cost/region pieces
        (the end-to-end adoption landed in M20.2, below).
- **Deliverable:** suite green (223 tests; snapshots byte-stable); no
  routing behavior change — [`edge_world`] and the whole dispatch
  ladder were untouched at M20.1. The classification was precomputed and
  dead-read by design; M20.2 flipped the direct-edge regime to
  [`whole_path_route`] and the same tests re-pin its live behavior.

**Blocked by:** nothing. **Blocks:** M20.2 (which consumed
[`whole_path_route`] / [`EdgeFrameWorld`] directly — done, see below).

### ✅ M20.2 — Whole-path search for direct internal edges

The plan's canary milestone, landed as scoped: **every unforced direct
(direct internal) edge is now routed by one whole-path A\* search**
([`whole_path_route`]) over the M20.1 frame world, and the search's result
is the route that is drawn. All twelve golden snapshots are **byte-identical
to the M20.1 state** — the search re-derives exactly the shapes the ladder
drew, which is the quality signal the plan asked for. The cross-boundary
regime (rep model, `cross_pieces_candidate`, `around_target_candidate`,
title detours, `route_lca` for cross-boundary spans) is untouched; M20.3
repoints it.

- [x] **The search is the router, not a trailing candidate.** For a direct
      edge the dispatch pushes the search result **first**, so it wins
      whenever it clears the hard world; the ladder's candidates stay behind
      it as the bounded fallback (a search that finds nothing leaves them to
      win — the M16 convention). The M20.1 skeleton had it as a *last*
      candidate, which could never win: the ladder's clean route always
      short-circuited the acceptance loop first, so the search fired on every
      edge and was chosen on none.
- [x] **The search re-derives the flat engine's shapes.** Two pieces were
      needed, both scoped to the whole-path search so no other route's graph
      or cost model moves:
      - [`Obstacles::jog_rows`] — the flat engine's own jog rows (the flow
        midpoints of its waypoint pairs) are seeded as extra tracks. A Z's
        total length is **jog-row-invariant**, so every row ties on length
        and bends; without these rows the search's shape for a simple
        diagonal was decided by tie-break alone.
      - [`Obstacles::port_graph`] — the start/goal are *points*, not tracks.
        A plain track graph connects only *adjacent* tracks, so a port's
        approach leg is always exactly one track gap ([`TRACK_OFF`] = 8) —
        short enough to disappear under the arrowhead, and identical for
        every jog row (which is why the tie could never be broken by a
        stub-length term). `port_graph` lets the start leave for, and the
        goal be entered from, any track visible along the shared row/column,
        making the stub length a real degree of freedom; the new
        [`MIN_TERMINAL_STUB`] / [`STUB_SHORTFALL_COST`] terms then keep the
        search off the degenerate 8 px approach.
- [x] **Forced sides stay on the ladder.** A forced `from=`/`to=` side binds
      the node port with M11/M14 semantics (the port lands on the requested
      side and escapes it by [`SIDE_ESCAPE`]; a contradicting request routes
      around). The search's free-choice port model cannot honor that, so the
      search is scoped to **unforced** direct edges; forced direct edges keep
      the M11 ladder. (Unscoped, the search shortened `sides.dgmr`'s mandatory
      24 px escapes to 8 px. M20.4 later re-tested this as the plan's "the
      forced port is simply where the search begins" and confirmed the
      rejection — see M20.4's forced-side bullet.)
- [x] **The obstacle model is the M20 one.** [`edge_world`] pushes the frame
      world's solid frames for direct edges (the plan's "unrelated frames may
      not be punctured" rule, by construction rather than cost tuning), and
      [`whole_path_route`] adds the ancestor bands as `borders` (parallel
      riding barred, perpendicular crossings at recorded waypoints) plus the
      `cross_bands` crossing cost ([`FRAME_CROSS_PENALTY`] per mid-run
      transversal). Self-loops keep their dedicated generator.
- [x] **Tests** (3 new; the two M20.1 placeholders for the live adoption are
      now real):
      - `whole_path_search_routes_direct_edges` — the classic mid-gap Z, with
        both terminal legs clearing the arrowhead's back-extent (10);
      - `whole_path_search_jogs_at_the_flat_engines_row` — the jog row is the
        gap midpoint, not the track the heap reaches first;
      - `whole_path_search_leaves_forced_sides_to_the_ladder` — a forced
        `from="top"` port stays on its side and escapes outward.
      The M16 dense-stress fixture and the M9/M16 regressions still pass; a
      node-interior cut count over the dense fixture is 0 with the search
      live (and 0 without — the search is not *needed* there, only
      equivalent, which is the point of this milestone).
- [x] **Removed:** the session's dead instrumentation (`SEARCH_WINS` /
      `SEARCH_FIRES` atomics, the `[m20-wave]` `eprintln!` probes and the
      `m20_wave_search_adoption_probe` test, whose counters never moved
      because the search could not win) and the now-superseded
      [`EdgeFrameWorld::track_seeds`] (the `Obstacles` seeding path replaced
      it; it was dead code).
- **Deliverable:** suite green (226 tests; snapshots byte-stable and
  deterministic across runs). Direct edges are drawn from one A\* route
  against the unified M20 obstacle model; the ladder remains only as their
  bounded fallback and as the live regime for cross-boundary and forced-side
  edges.

**Blocked by:** nothing. **Blocks:** M20.3 (which repoints the cross-boundary
regime at the same search and deletes the rep model).
### ✅ M20.3 — Whole-path search for cross-boundary edges; delete the rep model

The plan's big milestone, landed as scoped: **every unforced, non-self-loop
edge — direct and cross-boundary alike — is now routed by one whole-path A\*
search from source node port to target node port against the unified M20
obstacle model**, and the rep model is **deleted**. The containment tree's
only remaining routing role is the per-edge ancestor/solid classification.

- [x] **One router, one world.** [`whole_path_candidate`] feeds the search
      the edge's *node* ports (`from_port` → `to_port`) for both regimes —
      the plan's "one polyline from port to port" — and [`route_edge`] pushes
      its result first, so the A* route is what is drawn. [`edge_world`] is
      now the same world for every edge: node interiors, the edge's
      ancestor-frame bands (as `borders` + the crossing cost), its unrelated
      frames as fully-solid rects, and every group title's text rect as a
      solid obstacle ([`title_obstacle_rect`]).
- [x] **Titles are obstacles, not a special case.** The deleted title detour
      hand-built a jog below the title text; the search instead routes around
      the title's solid rect. Two layout consequences made that corridor
      exist: a titled frame's top inset keeps a route-wide strip below the
      text ([`TITLE_BAND_PAD`]), and a title-sized frame is wide enough to
      pass a route beside its title (the frame-width floor now includes the
      title's routing clearance plus one [`FRAME_PAD_X`]). Pinned by
      `no_edge_crosses_a_group_title` across all five samples.
- [x] **The search re-derives the ladder's cross-boundary rows.** A
      cross-boundary edge seeds [`Obstacles::jog_rows`] with the flow
      midpoints between consecutive *gates* along the run (node port, each
      chain frame's facing border, the other node port) and
      [`Obstacles::lane_row`] with the M9 lane pre-pass's assigned row — the
      row that keeps parallel edges through one gap apart. A
      [`LANE_DRIFT_COST`] preference (far above [`JOG_DRIFT_COST`]) makes the
      search land on the assigned lane wherever that row is routable; the
      row is only honored when it lies within the edge's own port span
      ([`LANE_ROW_SLACK`]), so a stale row cannot pull a route into an
      out-and-back spike.
- [x] **The row preference is charged per cross-run, not per track hop.**
      [`Obstacles::row_drift_cost`] is paid once at the bend that starts a
      cross-axis run. Charging per hop made the cost scale with how many
      tracks happened to lie along a run, which punished exactly the long
      runs the search should prefer.
- [x] **Both terminal legs get the stub preference.** The M20.2 term had
      only the goal paying [`MIN_TERMINAL_STUB`]; a cross-run jammed against
      the *source* port (a jog row 3 px below it) read as a kink in the box,
      so the source leg pays it too.
- [x] **Repair keeps the better route.** The M16 rip-up loop now keeps a
      reroute only when it is no longer *and* no more conflicting than the
      current polyline — a legal route is not shuffled into a longer one —
      and a route riding a peer's band never wins the comparison, so two
      riders cannot each "keep" themselves and survive every pass.
- [x] **Forced sides.** A forced `from=`/`to=` side still binds the node
      port with M11 semantics. A forced *cross-boundary* edge routes through
      the escape-and-ladder fallback ([`forced_cross_boundary_path`]), and a
      forced *direct* edge keeps the M11 ladder (M20.2's scope). The search
      is scoped to unforced edges; M20.4 settled the question the plan left
      open by measuring the alternative and rejecting it. Pinned by
      `forced_cross_boundary_ports_are_honored_by_the_search` (all four
      sides) and the M11 forced-side suite.
- [x] **Deleted** (the milestone's headline): `cross_pieces_candidate`,
      `around_target_candidate`, `try_around_target_route`,
      `cross_boundary_path`, `CrossPieces`, `stub_for`, `build_stub`,
      `StubEnd`, `stub_world`, `reroute_stub_span`, `blocked_beyond_title`,
      `mismatched_stub`, `mismatched_rep_cross`, `mismatched_bend`,
      `MismatchBend`, `inner_far`, `sub_intervals`, `clamped_escape`,
      `into_param`, `record_chain_crossings`, `title_detours`,
      `TitleDetour`, `title_detour_clear_x`, `lca_segment`,
      `side_perpendicular`, `refresh_rep_crosses`, `ctx_with_crosses`'s rep
      half, `cross_boundary_reach`, and the M16 frame rep-crossing fan pass.
      With them go `EdgeCtx`'s rep-port fields (`from_frame_side`,
      `to_frame_side`, `from_mismatched`, `to_mismatched`, `from_rep_cross`,
      `to_rep_cross`, `around_target`, `lca_dir`, `rep_*_rect`) and the
      constants that only they read (`STUB_JOG_CLEARANCE`,
      `CROSS_FRAME_PAD`, `TITLE_LEFT_ENTRY_MIN`, `APPROACH_MIN`,
      `AROUND_CLEAR`), plus the helpers only they used (`port`,
      `side_flow_coord`, `port_flow`, `line_rect_exit`, `orthogonalize`,
      `cross_range`, `Geometry::children`/`title_width`/`title_text_rect`,
      `Geometry::frame_children`, the `Obstacles::push_frame`/
      `push_segment` leftovers). `CROSS_FRAME_PAD` retires in favour of
      [`TITLE_BAND_PAD`], which serves the same purpose (a route-wide strip
      below the title) without the per-chain-frame over-approximation.
- [x] **Tests.** New: `cross_boundary_edges_route_by_the_whole_path_search`
      (one port-to-port polyline, a single frame crossing),
      `no_edge_crosses_a_group_title` (all samples),
      `forced_cross_boundary_ports_are_honored_by_the_search` (all four
      sides), `whole_path_search_never_punctures_an_unrelated_frame`,
      `cross_boundary_routes_are_deterministic`,
      `every_edge_lands_on_its_endpoint_boundary_on_the_samples`, and
      `cluster_and_tencent_edges_do_not_overlap_or_pass_through_nodes` (the
      two deepest cross-boundary samples join the no-overlap/no-pass-through
      gate). Deleted
      with the code they pinned: the rep-crossing fan test, the LCA↔stub
      seam test, the four title-detour tests, the around-target pair, the
      aligned-entry pair, the sibling-pierce test, the S19b mismatched stub
      test, and the two rep-port waypoint-count tests
      (`cross_boundary_edge_connects_two_frames`,
      `cross_boundary_edge_routes_through_frame_connection_point` — their
      intent, "crosses the frame at a recorded point", lives on in
      `cross_boundary_frame_points_preserve_group_direction` and
      `cross_boundary_edge_crosses_each_nested_frame`). The M9/M16/M17 regression gates (dense stress, phase-1
      permutation invariance, no-overlap/no-pass-through on every sample,
      label resolution) all stay green unchanged.
- [x] **Docs.** The `layout.rs` module docs now describe the one-router
      architecture; `AGENTS.md`'s pipeline, module map, and shared-constant
      notes are updated, and its "whole-path search is the router" invariant
      states the M20.3 scope explicitly.
- **Deliverable:** suite green (214 tests + 7 ignored inspection helpers;
  snapshots regenerated deliberately — six changed: `cluster`, `finance`,
  `group_style`, `infra`, `sides`, `subdirection`). Every route is one
  search polyline; the six untouched snapshots (`cycle`, `diamond`,
  `isolated_nodes`, `left_right_chain`, `shapes_styles`, `simple_chain`) are
  byte-identical, which is the quality signal: simple shapes still come out
  exactly as before. The visible churn is the title band (`TITLE_BAND_PAD`
  adds 14 px of frame height) and the cross-boundary rows landing on the
  search's own choices instead of the ladder's hand-built ones.

**Blocked by:** nothing. **Blocks:** M20.4 (done — the fan, pockets, and the
repair loop are re-grounded there).

### ✅ M20.4 — Frames bands, fans, pockets, repair loop re-grounded

The plan's re-grounding milestone. Its headline is the **frame-crossing
fan**: the M9 port-fan idea, applied to the *ancestor frame sides* the
whole-path search crosses. Everything else the milestone asked for turned
out to be either already true (the pockets, the repair loop) or a
deliberate non-goal (grounding forced sides on the search).

- [x] **The frame-crossing fan ([`separate_frame_crossings`]).** The
      whole-path search chooses where each edge crosses its ancestor
      frames' border bands, and *every* crossing is a designated waypoint
      ([`seg_clear_border`] exempts a segment end on the band centerline)
      — so two edges entering one frame side both take the side's cheapest
      row and land on **one coordinate**. The repair loop then negotiates
      them apart, and on a fan into one frame that means pushing one edge
      all the way around it. The pre-pass allocates each shared frame
      side's crossing coordinates up front (ordered by each edge's
      *inside* endpoint's column, so a lone edge keeps the descent it would
      draw anyway) and hands the search its own coordinate to prefer. It is
      a preference, never a bar: a blocked crossing row loses to a clean
      alternative.
- [x] **Seeded as tracks, preferred as a cost.** The assigned coordinate
      joins the search's track graph on the side's own axis
      ([`Obstacles::crossing_rows`]: a Top/Bottom side's crossing is an `x`
      track, a Left/Right side's a `y`) and pays [`CROSSING_DRIFT_COST`]
      per pixel of drift at the bend that starts the run — the
      [`Obstacles::lane_row`] pattern, so the charge is per *run start* and
      does not scale with how many tracks lie along the run.
- [x] **A band-thickness exemption for the graph.** Seeding a crossing
      coordinate inside a band's 1.5 px thickness split one perpendicular
      crossing into two hops, the second of which began *beside* the
      centerline and so failed the designated-crossing exemption — which
      made the search detour around the frame instead of entering at its
      crossing. [`in_band`] extends the exemption to a segment end anywhere
      in the band (perpendicular exit), which is what makes the seeded
      coordinate reachable at all.
- [x] **Only frames the span straddles are fanned.** A frame containing
      *both* ports is crossed twice (in and out) and its crossings are the
      search's business; a frame containing neither is not on the span. The
      side fanned is the one the port-to-port segment *meets first*
      ([`frame_entry_side`], geometric), not "the side facing the source
      along the dominant axis" — a wide frame's centre is far from the
      edge's own column, so a centre comparison picks a vertical side for an
      edge that plainly enters through the top.
- [x] **Pockets needed no repointing.** [`pocket_rects`] already derives
      each edge's reserved approach corridors from that edge's *current*
      polyline (the first and last terminal legs, truncated to
      [`PORT_RIGHT_OF_WAY`]), so a whole-path search output is treated
      exactly like any other route. Verified rather than changed.
- [x] **The repair loop needed no repointing either.** It already reroutes
      whole paths through the one dispatch point ([`route_edge`]) and scores
      candidates with [`route_score`]. What M20.4 changed is that it now
      usually has *nothing to do*: the pre-passes seat the shared resources
      up front. Measured — the nested five-source fan fixture goes from
      1597.7 px (with repair) to 1457.3 px (phase 1 = final) with the fan
      live, i.e. the fan removes 140.4 px of repair detour.
- [x] **Forced sides are deliberately *not* grounded on the search.** The
      plan's sketch ("the forced port is simply where the search begins")
      was implemented and measured, and it is a regression: the search's
      cost model has no term for "leave the box along the requested side by
      [`SIDE_ESCAPE`] before anything else", so grounding a forced direct
      edge on it drew shorter but wrong shapes (a forced `from="top"` escape
      collapsed to a stub barely longer than the arrowhead, and
      `examples/sides.dgmr`'s forced excursions lost their deliberate
      clearance — a −157 px "improvement" that wrapped a frame). M11's
      escape-and-ladder already encodes exactly what the attribute asks
      for, so forced direct edges keep it and forced cross-boundary edges
      keep the escape-and-ladder fallback. Recorded in [`route_edge`]'s
      scope note and the module docs.
- [x] **Tests** (6 new): `frame_crossing_fan_seats_nested_frame_crossings`
      (the five-source nested fan: every frame side's crossings
      ≥ [`FAN_SEP`] apart *and* phase 1 = final, which fails with
      "the repair loop added 140.4 px" if the pre-pass is removed),
      `frame_crossing_fan_is_what_seats_the_columns` (the pre-pass driven
      directly: three identical *wants* come back separated; a lone crossing
      gets no entry), `frame_crossing_fan_is_per_side`,
      `frame_crossing_fan_is_permutation_invariant` (M17 still holds — the
      pre-pass sorts on a content key), `frame_crossing_fan_spreads_out_of_span_sources`,
      and `dense_fan_into_one_frame_needs_no_repair` (the dense-stress
      re-baseline: eight sources into eight targets through one frame, phase
      1 already conflict-free). The M9/M16/M17/M20.1–M20.3 gates all stay
      green unchanged.
- **Deliverable:** suite green (220 tests + 7 ignored inspection helpers).
      **One snapshot changed: `cluster`**, and deliberately: the three
      `lb -> n*` edges into the nested "Node Group" frames had all collapsed
      onto one crossing column, and the old snapshot shows the repair loop's
      fix — two of the three swinging out to `x ≈ 507`/`537` and back, 678.6
      px of route that the fan now avoids entirely (`4022.8` → `3383.6` px
      of total edge length). The other eleven snapshots are byte-identical,
      including `finance`, which the first draft of the fan had perturbed
      (the "inside endpoint's column" and "first side met" rules are what
      bring it back to exactly the committed geometry).

**Blocked by:** nothing. **Blocks:** M20.5 (labels ride these polylines).

### [ ] M20.5 — Labels re-tuned (M13 still holds its contract).
(see ./plan-for-whole-path-maze-routing.md)
### [ ] M20.6 — Deletion & doc pass.
