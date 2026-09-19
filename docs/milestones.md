# Milestones

Tracking doc for diagrammer. The grammar is specified in
[`grammar.md`](grammar.md); this file records what's done, what's next, and
how the remaining work depends on itself.

**Status legend:** ✅ done · ▶ next · ⬜ planned

**Current position:** M0, M1, M2, M3, M4, M5, and M5.5 are complete. **M6 (render shapes, styles, and colors) is next.**

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

**Blocked by:** M2. **Note:** orthogonal edges are deferred (see M7).

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
`snapshots/infra.svg` and `snapshots/subdirection.svg` regenerated. Segments
may still be diagonal; M7 makes them orthogonal.

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

## ▶ M6 — Render shapes, styles, and colors (already parsed)

Most of this is already parsed in M1; this milestone is purely rendering work
once the SVG pipeline exists.

- [ ] Cylinder shape.
- [ ] Edge line styles: `dotted`, `dashed`, `thick`.
- [ ] Node `color` (stroke) and `fill` (interior); edge `color`.
- [ ] Edge labels (positioned along the edge).

**Blocked by:** M3. Can interleave with M4/M5.

## ⬜ M7 — Orthogonal edge routing

Upgrade edges from diagonal to right-angle routing typical of infrastructure
diagrams.

- [ ] Orthogonal router (e.g. channel-based) producing bend points.
- [ ] Arrowhead orientation at segment end.
- [ ] Snapshot tests updated.

**Blocked by:** M3, M5.5. Independent of M4/M5; can be done after M6.

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
