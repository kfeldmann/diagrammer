# AGENTS.md — diagrammer

Read these docs **first**, before touching code — they tell you where the
project is and what's next without reading source:

- [`docs/milestones.md`](docs/milestones.md) — status + plan (what's done,
  what's next, dependency graph). The **Current position** line at the top
  says which milestone is in flight.
- [`docs/grammar.md`](docs/grammar.md) — the v0 input-language spec the
  parser/lexer are built against.
- FUTURE: [`docs/checker-spec.md`](docs/checker-spec.md) — the routing checker spec
  (agent-based routing improvement work; read before touching
  `tools/checker/` or any `experiments/` prototype).

## What this is

A small CLI that reads a Mermaid-like DSL and emits self-contained,
GitHub-renderable SVG. The core reason it exists: Mermaid/dagre **ignore
per-`subgraph` layout direction when edges cross group boundaries** —
diagrammer escapes that by laying each group under its own direction and routing
cross-boundary edges through frame connection points without disturbing the
groups' internal arrangement.

## Pipeline (end to end)

```
source string
  → lexer.rs        tokens (ids w/ hyphen rule, quoted strings, # comments)
  → parser.rs       raw AST (RawDiagram: every decl as written, incl. dups)
  → resolve.rs      validated Diagram (dedup nodes, strict attrs, group
                    membership + containment tree, per-group direction)
  → layout.rs       Layout (Sugiyama flat engine + compound recursion +
                    whole-path maze routing for every edge (M20.3) + M9
                    edge separation & lane assignment + M13 label placement;
                    forced `from=`/`to=` sides bind the endpoint node only)
  → decorate.rs     post-routing page furniture (M21.3): the `decorate` pass
                    runs after `assemble` has finished routing and grows the
                    canvas for `border`/`titleblock`/`textblock`; no-op when
                    none is declared
  → render/svg.rs   self-contained SVG string
  → main.rs         CLI: `diagrammer <in.dgmr> [-o <out.svg>]`
```

`ast.rs` holds both layers: the `Raw*` syntactic types and the resolved
`Diagram`/`Node`/`Edge`/`Group`, plus the page-furniture statements
(`border`/`titleblock`/`textblock`, M21.1). The page furniture is
**decoration only** — not a node, edge endpoint, or group member, absent from
the layout graph, the obstacle model, and the routing vocabulary — so it
resolves into `Diagram::{border, title_block, text_blocks}` and nothing
downstream is forced to care. `resolve` validates it (uniqueness, the
lower-right/title-block corner conflict, the open-but-duplicate-free
title-block keys, a positive `textblock` `width`, a non-empty body, `line`
against `Style`) and hands on the validated style values, which are the
single source of truth for `decorate.rs`'s placement pass — the renderer draws
what `resolve` validated instead of re-deriving (and possibly disagreeing
about) a color or a line style. The CLI summary also appends `; page: …`
**only when something is present**, which is what keeps every pre-M21 summary
(and every snapshot) byte-identical.

`text.rs` measures labels against the baked
DejaVu Sans metrics in `metrics_table.rs` — generated from the embedded fonts
by `tools/gen-metrics-table` and verified against them by tests
(deterministic across machines). Since M21.2 the table carries **two** faces
(Regular and Bold) behind one `Face` struct: `text::measure` is Regular and
stays bit-identical for every pre-M21.2 caller, while `text::measure_run(text,
size, Weight::Bold)` measures an inline bold run with the bold advances
(12–23% wider — Regular metrics would overflow a text block). Italic has no
table and no `Weight` variant: DejaVu Sans Oblique has Regular's advances and
em box for ASCII and Latin-1 except four glyphs (≤1.4%), so it measures as
Regular by design. Both faces share an em box, asserted at compile time, so
`line_height` needs no weight term.
`error.rs` is an offset-carrying `Error` enum → 1-based line numbers.

## src/ module map

```
src/
  main.rs        CLI: arg parsing, run(), summary vs. -o SVG output
  lexer.rs       tokenizer
  parser.rs      winnow parser → raw AST (incl. the page-furniture blocks,
                 the group-body rejection, and the one raw-body recognizer)
  ast.rs         Raw* + resolved Diagram/Node/Edge/Group + Shape/Style/Direction enums
  resolve.rs     raw → validated Diagram (dedup, attrs, group membership,
                 page-furniture validation)
  layout.rs      ★ biggest file: flat Sugiyama engine + component packing
                 (M15: edge-less items follow the scope's direction) +
                 compound (per-group direction) + M20 whole-path maze routing:
                 one A* from source node port to target node port over one
                 obstacle model (node interiors, ancestor-frame bands, solid
                 unrelated frames, solid group titles) for every unforced
                 edge — direct and cross-boundary alike (M20.3 deleted the
                 rep model: rep ports, frame-crossing fan, within-frame
                 stubs, title detours, around-target wrapper, M14
                 coupled/decoupled duality) + M9/M20.4 edge separation
                 pre-passes — port fan, lane assignment, and the
                 frame-crossing fan (`separate_frame_crossings`: edges into
                 one ancestor frame side get distinct crossing columns; the
                 search honors all three as row preferences) + sealed-port
                 fallback (a lone side's port slides when its escape pocket
                 is sealed) + M13 label placement & label-aware fan/lane
                 spacing (M20.5: candidate anchors are offsets along each
                 polyline's *visual runs* — collinear waypoints merged — and
                 the title obstacle is the router's grown one) + M16 global
                 routing pass (a simultaneous pass against the shared
                 obstacle world — no peer segments — then a bounded
                 rip-up-and-repair loop with soft peer occupancy and reserved
                 port pockets, both derived from each edge's current polyline
                 so they needed no M20.4 repointing) + M17
                 union-merge lane grouping with content-keyed (endpoint-id)
                 sort tie-breaks, so phase-1 routing is invariant under edge
                 permutation
                 → Layout (plus `Layout::{border, title_block, text_blocks}`,
                 filled in by the post-routing `decorate` pass)
  decorate.rs    M21.3: the post-routing page-furniture pass — sizes and places
                 `border`/`titleblock`/`textblock` against the finished canvas
                 (band reservation, the title table's geometry, text-block
                 inline markup + wrapping) and grows the canvas; a no-op when
                 none is declared
  text.rs        label measurement against the baked metrics tables;
                 `measure` = Regular, `measure_run(.., Weight)` for bold
  metrics_table.rs  GENERATED baked DejaVu metrics (Regular + Bold faces) —
                 never hand-edit; regenerate: cargo run -p gen-metrics-table
  error.rs       Error enum + line_of()
  render/
    mod.rs       `pub mod svg;`
    svg.rs       Layout+Diagram → self-contained SVG (page furniture first,
                 then boxes, cylinders, edges, edge labels at their
                 layout-given anchors, per-color arrowhead markers)
```

## Build, test, run

```bash
cargo test                            # full suite (incl. golden snapshots)
cargo test -- --skip snapshot         # logic only, faster feedback
UPDATE_SNAPSHOTS=1 cargo test         # regenerate golden .svg files (commit them)
cargo run -p gen-metrics-table       # regenerate src/metrics_table.rs (font changes; commit it)
cargo run -- examples/infra.dgmr -o out.svg   # render one diagram
cargo run -- examples/infra.dgmr               # print a validation summary
cargo clippy --all-targets            # must be warning-free (dead_code = deny)
cargo doc --no-deps                   # must be warning-free (rustdoc links)
```

**The snapshot harness is non-obvious:** `src/render/svg.rs` zips each diagram
against a golden file in `snapshots/`. When rendering output intentionally
changes, regenerate with `UPDATE_SNAPSHOTS=1` and commit the updated `.svg`
files (don't hand-edit them).

**Release builds** are Docker-based and live in `build/`: one script per
platform (`build-linux-glibc`, `build-linux-musl`, `build-mac`,
`build-mac-x86_64`, `build-windows`), sharing `build/debian.Dockerfile` (glibc
+ macOS and Windows cross via `cargo-zigbuild`) and `build/alpine.Dockerfile`
(musl). Each mounts the project at `/work` and runs as the host UID:GID. See
`build/README.md`.

Windows is `x86_64-pc-windows-gnu`, not `-msvc`: zig can't drive the MSVC
linker (`/NOLOGO: unrecognized file extension`), and the gnu target
statically links the MinGW runtime, so the `.exe` has no `libgcc_s_seh-1.dll`
/ `libwinpthread-1.dll` beside it — its only imports are OS components. It is
a console-subsystem PE32+ binary (stdout/stderr and exit codes work in
`cmd`/PowerShell) and ships **unsigned**; `build/README.md` covers what that
costs and the signing options.

## Invariants — don't break these

- **Index correspondence.** `Layout.nodes`/`Layout.edges`/`Layout.groups`
  correspond **by index** to `diagram.nodes`/`diagram.edges`/`diagram.groups`
  in declaration order. The renderer `zip`s them directly. Any new output
  keyed by id must preserve this. (Edge-label anchors ride on `EdgePath` —
  `label_at: Option<(f32, f32)>`, M13 — for exactly this reason; don't move
  them into a separate keyed collection. M21.3's `Layout.text_blocks`
  extends the invariant: `text_blocks[i]` is `diagram.text_blocks[i]`, and the
  renderer zips them.)
- **Canonical top-down space, then transform.** All layout math runs in
  canonical top-down space (rank grows down, order is horizontal), then a
  rigid transform maps rects + waypoints into the requested direction. The
  same transform applies to node rects and edge waypoints, so ports stay
  glued to the correct side of each box in all four directions. Shape/label
  math that reasons about "up/down" must account for this (e.g. the cylinder
  cap radius and label shift are direction-agnostic because they live in
  rendered box space, not canonical space).
- **Page furniture is decoration only, and unused ⇒ byte-identical (M21).**
  `border`/`titleblock`/`textblock` are not graph participants: no block is a
  node, an edge endpoint, or a group member, and none joins the layout graph,
  the obstacle model, or the routing vocabulary. They live in `Diagram`
  (`border`, `title_block`, `text_blocks`) and in `Layout`
  (`border: Option<Rect>`, `title_block: Option<Rect>`, `text_blocks:
  Vec<BlockRect>`), filled in by `decorate.rs`'s `decorate` pass, which
  `layout()` runs **after** `assemble` has finished routing — never before, so
  no block can influence a route. The pass early-returns when none of the three
  is declared, which is the acceptance property: a diagram using none of them
  renders **byte-identical**. That is why the model is `Option`/empty by
  default, why the CLI summary only appends its `; page: …` suffix when
  something is present, and why the renderer emits no page groups at all for
  such a diagram. The border is drawn on the fixed page *inset* (28 px, outside
  the 20 px content margin), so `border` present-vs-absent changes no
  coordinate — it only strokes a rectangle the inset already reserved; the
  same inset positions the title block and every text block, so all three are
  decoupled from each other and from `border`. Don't make a block a layout
  participant (that was considered and dropped for text blocks: an anchored
  block is positioned from the finished canvas, so making it an endpoint
  would need a bounded place→route→re-place loop and a new endpoint
  representation through `EdgeCtx`/`FrameClass`/`Obstacles`/`LabelWorld`).
- **GitHub-renderable output.** No `<script>`, no external references, no CSS,
  only inline attributes. This has already forced decisions: e.g. one
  arrowhead `<marker>` per distinct edge color with a hard-coded `fill`,
  because `currentColor`/`context-stroke` don't inherit into markers reliably
  and GitHub sanitizes — don't "simplify" it back to a single currentColor
  marker. Each edge line is also shortened at the target end (by
  `ARROW_BACKOFF` in `render/svg.rs`) so the stroke tucks under its same-color
  arrowhead — a thick line's round cap would otherwise poke past the tip and
  read as a blunt arrow — and the marker's `refX` is reduced by the same
  amount so the tip stays on the node boundary. Text uses the
  `FONT_FAMILY` stack in `render/svg.rs` — `Arial, Helvetica, Liberation
  Sans, DejaVu Sans, sans-serif` — leading with the metric-compatible trio
  so all viewers render near-identical glyphs; don't reorder it back to
  DejaVu-first (that would make Linux viewers see ~10% wider text than
  Windows/macOS ones). Measurement stays DejaVu-first (below), the wider
  font, so boxes never under-size for any stack member.
- **No dead code, and no `#[allow(dead_code)]`.** `Cargo.toml` sets
  `[lints.rust] dead_code = "deny"`; a crate-root `#![allow(dead_code)]` in
  `main.rs` (removed at M20.6) had hidden five milestones of orphaned
  scaffolding, so an unused `fn`/`struct`/field now fails the build instead
  of warning. Don't add an allow to dodge it — delete the code, or wire it
  up. Field-level allows are equally discouraged. Likewise, production code
  carries **no env-driven debug switches**; the only sanctioned ones are the
  `#[ignore]`d inspection tests (which read `DIAG_SRC`).
- **Deterministic output.** `fmt()` formats floats to exactly 2 decimals; text
  is measured against the baked metrics of the **embedded** DejaVu Sans
  Regular and Bold faces (`src/metrics_table.rs`), never a system font. Snapshots are byte-stable across
  machines — don't introduce nondeterminism (random ids, hashmap iteration
  order leaking into output, system fonts). Edge-label placement (M13) is a
  fixed-order greedy pass plus one refinement pass over candidate anchors —
  keep it that way (no randomness, no iteration-order dependence).
- **2-D distance goes through `portable_hypot`, never `f32::hypot`.** This is
  the determinism invariant's sharpest edge: `f32::hypot` forwards to the
  platform's libm `hypotf`, which Rust documents as "varies by platform, Rust
  version, and can even differ within the same execution". musl computes
  `z * sqrtf((double)x*x + (double)y*y)`; mingw-w64's narrows a double
  `_hypot`. They differ by one ulp on ~13% of plausible layout deltas, and
  because `fmt()` rounds to two decimals a 1-ulp difference flips a printed
  coordinate about once per 1,500 samples — so a Windows or macOS build would
  emit different SVGs than the Linux-built goldens while every test still
  passed (text metrics are table-driven and never touch libm).
  `portable_hypot` sums in `f64` and takes `f64::sqrt`, which IEEE-754
  requires to be correctly rounded and hardware implements directly, so it is
  bit-identical everywhere. **`f64::hypot` is not a fix** — it is libm too and
  reproduces the same divergence. All twelve call sites go through it
  (`layout.rs`'s edge lengths, port escapes and label-run selection, plus
  `render/svg.rs`'s arrowhead backoff and label-segment pick — those two move
  *drawn* coordinates). Pinned by `layout::tests::portable_hypot_*`, which
  assert exact bit patterns for inputs where the libms disagree, and by
  `layout::tests::no_production_code_calls_hypot`, a source-level guard that
  fails on any `.hypot(` in production code. The guard matters because the
  drift is invisible on any single machine — the snapshots still pass on the
  machine that regenerated them.
- **Input files are normalized at the read boundary (BOM + CRLF).**
  `main::normalize_source` strips a leading UTF-8 BOM and rewrites `\r\n` to
  `\n` — the two artifacts a Windows editor or PowerShell (`>` /
  `Out-File`) leaves behind. The BOM matters for correctness: the lexer has no
  rule for `U+FEFF`, so a BOM'd file otherwise dies with the useless
  `error at line 1: diagram header`. The CRLF rewrite is not about the
  grammar (`lexer::eol` already accepts `\r\n`); it keeps a literal `\r` from
  a quoted `"a\r\nb"` label out of the SVG `<text>` and out of
  `text::measure`'s width. Only a *leading* BOM is stripped (an interior
  `U+FEFF` is label content) and a lone `\r` is left alone (the grammar never
  sanctioned bare-CR line endings). Keep normalization here, at the one place
  bytes become a `str`, rather than teaching the lexer about BOMs.
- **The whole-path search is the router (M20.3).** Every *unforced*
  non-self-loop edge — direct (M20.2) and cross-boundary (M20.3) alike — is
  routed by one `whole_path_route` A* search from source node port to target
  node port over one obstacle model, and `route_edge` pushes that result
  **first**, so the A* route is what is drawn. Things that keep this from
  silently regressing:
  (a) the caller must seed `Obstacles::jog_rows` (the flat engine's own jog
  rows) and set `Obstacles::port_graph`, or the search cannot re-derive the
  flat engine's shapes (a Z's length is jog-row-invariant, so without them
  the shape is decided by tie-break); for a cross-boundary edge the
  mid-stub/rep-port gate rows and `Obstacles::lane_row` (the M9 lane
  pre-pass's assigned row) join them.
  (b) the obstacle model is the M20 one — `edge_world` builds it for *every*
  edge: node interiors, the edge's ancestor-frame bands (crossable, never
  rideable), its unrelated frames fully solid, every group title's text rect
  solid. Don't reintroduce per-edge-kind world variation.
  (c) the search is scoped to **unforced** edges only — a forced
  `from=`/`to=` side keeps M11 semantics (port on the requested side,
  `SIDE_ESCAPE` outward) which the search's free-choice port model cannot
  honor. A forced *cross-boundary* edge routes through the
  escape-and-ladder fallback (`forced_cross_boundary_path`); a forced
  *direct* edge through the M11 ladder. **Settled at M20.4:** grounding
  forced sides on the search (the plan's "the forced port is simply where
  the search begins") was implemented and measured — it is a regression
  (shorter but wrong shapes: the outward `SIDE_ESCAPE` collapses to an
  arrowhead-length stub), so don't re-attempt it without adding a cost term
  that encodes the escape.
  The rep model (rep ports, frame-crossing fan, within-frame stubs, title
  detours, around-target wrapper, M14 coupled/decoupled duality) is
  **deleted** — don't resurrect it; the containment tree's only routing role
  is the ancestor/solid classification (`FrameClass`).
- **The separation pre-passes are what make the search's output legible
  (M9/M16, re-grounded at M20.4).** Three deterministic pre-passes run
  before any search and feed it *preferences* (never bars): the port fan
  (`separate_ports`), the lane assignment (`assign_lanes`), and the
  **frame-crossing fan** (`separate_frame_crossings`). The last one is
  load-bearing and easy to lose: *every* ancestor-band crossing is a
  designated waypoint, so it costs the same as any other, and without an
  allocated coordinate two edges entering one frame side both pick the
  side's cheapest row — they collapse onto one column and the repair loop
  pushes one all the way around the frame (678.6 px on `cluster`, 140.4 px
  on the nested-fan fixture). The fan's pieces that must stay together:
  (a) the *inside* endpoint's column is the desired coordinate (a lone edge
  then keeps the descent it would draw anyway), and the side fanned is the
  one the port-to-port segment meets first (`frame_entry_side`) — "the side
  facing the source along the dominant axis" picks a vertical side for an
  edge that plainly enters through the top of a wide frame, and perturbs
  `finance`;
  (b) the assigned coordinate is seeded as a track **on the side's own
  axis** (`Obstacles::crossing_rows`) and preferred by
  `CROSSING_DRIFT_COST`; seeding the wrong axis drops a track inside the
  band's thickness and the crossing becomes unreachable;
  (c) `seg_clear_border`'s designated-crossing exemption must accept a
  segment end anywhere in the band (`in_band`), not only on the exact
  centerline, or a seeded crossing splits into two hops and the second is
  rejected.
  `pocket_rects` and the M16 repair loop needed no repointing: both already
  work off each edge's *current* polyline / the one dispatch point.
- **Labels are placed along *visual runs*, not raw waypoints (M20.5).** The
  router marks every ancestor-band crossing, lane row and jog row as an
  explicit waypoint, so one drawn straight run arrives as a chain of
  collinear segments (`cluster`'s `lb -> n1` has eight waypoints for three
  visual runs). M13's candidate generator is segment-local — longest-segment
  first, with the knockout clamped inside the segment — so reading raw
  segments parks labels at taps of fragments, hard against bends. `visual_runs`
  merges consecutive collinear *and forward-going* segments, and both
  `label_candidates` and `fallback_label_anchor` work in that vocabulary.
  The forward-going requirement is load-bearing: an out-and-back spike left
  on a collinear stretch by the repair loop must stay two runs, or the label
  is offered a zero-length or reversed one. Don't "simplify" the merge to
  plain collinearity, and don't fold it back into the raw-waypoint reading.
- **A knockout may cross a frame outline but must not be centered on it
  (M20.5).** M13.1 deliberately keeps group borders out of `LabelWorld`
  (a label may cross a group's outline; only the title text is an
  obstacle). Merging visual runs moves run midpoints onto exactly the frame
  edges a run crosses, which buries the drawn line under a white square, so
  the engine charges `FRAME_CENTER_COST` when the knockout's **center**
  falls in a frame edge's border band (`rect_covers_line`, fed by
  `LabelWorld::frame_edges`). It is a *preference*, sized above the midpoint
  term and below the graze term — keep it that way; turning borders into
  obstacles would retract M13.1.
- **One title rect, shared with the router (M20.5).** `place_labels` scores
  knockouts against `title_obstacle_rect` — the same grown rect (measured
  glyph box + viewer-fallback margin) that `edge_world` pushes as a solid
  obstacle — not the bare `title_text_rect`. Two different title boxes let a
  knockout sit in space the search had reserved for the fallback-font case
  M20.3 exists to protect. `assert_labels_resolved` asserts against the
  grown rect for the same reason.
- **Measurement is baked and verified.** `src/metrics_table.rs` is *generated*
  — never hand-edit it; regenerate with `cargo run -p gen-metrics-table`
  (and commit the result) when the bundled font changes. It holds two faces
  (M21.2): one `Face` struct with a `REGULAR` and a `BOLD` const, emitted by
  the same generator code path, so the lookup semantics cannot drift between
  them. `text::measure` is exactly `measure_run(.., Weight::Regular)` and
  must stay **bit-identical** for every existing caller (node labels, frame
  titles, edge labels are all Regular — the whole pre-M21 layout path); only
  inline-bold text-block runs use `Weight::Bold`. **Italic is Regular, and
  that is deliberate** — Oblique shares Regular's em box and, across ASCII
  and Latin-1, its advances except for four glyphs (`« » Þ æ`, ≤1.4%), so a
  third ~100 KB table would not pay for itself, and v0 already measures the
  widest plausible stack member (a test pins the ASCII + Latin-1 claim). The
  `font_verification` tests in `text.rs` prove **each** face is bit-identical
  to live `ttf-parser` lookups over the whole Unicode range — keep them green
  (they are the slowest tests in the suite; running both faces costs ~40 s
  instead of ~20 s). Two load-bearing quirks (pinned by those tests): pixel
  scaling is `font_size / (ascent − descent)` (the em box, **not**
  `units_per_em`), and kerning is *first horizontal subtable wins* with
  zero-valued pairs treated as absent. Both faces share an em box, which is
  why `line_height`/`Metrics::height` carry no weight term — a compile-time
  `const _` block in `text.rs` asserts it, so a regenerated table that
  drifted fails the build.

## Shared constants across the layout/render boundary

These are defined in `layout.rs` and read by (or constrained against)
`render/svg.rs` — keep them in sync (changing one without the other silently
breaks geometry):

- `layout::FONT_SIZE = 14.0` — node label size; the renderer sizes `<text>`
  to match the boxes laid out from these metrics.
- `layout::EDGE_LABEL_SIZE = 12.0` and `layout::LABEL_PAD = 3.0` — edge-label
  font size and knockout-rect padding (M13). The label placement engine in
  `layout.rs` (`label_box`) sizes the very knockout rects `render/svg.rs`
  draws around `EdgePath.label_at` from these; both sides must agree or the
  knockout drifts off its anchor.
- `layout::CYL_RY = 5.0` — cylinder elliptical-cap radius; layout sizes the
  cylinder box to include it (`cyl_height`), the renderer draws the caps at
  this radius.
- `layout::FRAME_CENTER_COST = 4.0` — the label engine's flat preference
  against centering a knockout on a group frame's drawn outline (M20.5). It
  sits above `TAP_W` (6.0 at a segment's end, 2.0 at a third, so an adjacent
  tap wins) and below `OFFSET_W` (8.0, the graze penalty, so nothing is
  shoved sideways merely to dodge an outline). It is *not* an obstacle:
  M13.1 keeps a knockout crossing a frame border legal, and only the
  knockout's center is tested (`rect_covers_line`). Raising it above
  `OFFSET_W` would start overriding the graze escape; removing it buries
  frame outlines under label squares wherever a run's midpoint lands on a
  border.
- `layout::FRAME_TITLE_X = 10.0`, `layout::FRAME_TITLE_TOP = 4.0`, and
  `layout::FRAME_TITLE_FONT_SIZE = 12.0` — the inset, top offset, and font
  size of a group's title text. Layout reads these to build the title's
  **routing obstacle** (`title_obstacle_rect`, M20.3) — the solid rect the
  whole-path search keeps clear, so it routes around a title instead of
  through it — and to build the *same* rect as the label engine's title
  obstacle (M20.5: one box for both, so a knockout cannot sit in space the
  search reserved for a viewer's wider fallback face). They must match
  `render::FRAME_TITLE_X` (10.0),
  `render::FRAME_TITLE_Y` (4.0), and `render::FRAME_TITLE_SIZE` (12.0), else
  the search would clear the wrong box and a route could cross the drawn
  title.
- `layout::TITLE_BAND_PAD = 14.0` — the strip kept below a titled frame's
  title text (added to the frame's top inset). It is what makes a within-frame
  corridor under the title exist, so the whole-path search can pass under a
  title instead of around the frame. Sized above `ROUTE_PAD` plus a track gap.
- `layout::FAN_SEP = 10.0` — the minimum gap between two fanned ports on a
  node side *and* between two fanned crossings on an ancestor frame side
  (M20.4). Both fans use it as the floor their label-aware separation
  (`label_pair_sep`) starts from; shrinking it makes fans read as one line.
- `layout::MIN_TERMINAL_STUB = 12.0` and
  `layout::STUB_SHORTFALL_COST = 25.0` — the whole-path search's port-stub
  preference (M20.2). A Z's total length is jog-row-invariant, so every track
  row ties on length and bends; without a stub term the search jogs one track
  gap (8 px) off the target, which hides under the arrowhead. Both terminal
  legs (source and target) pay it, so a cross-run cannot jam against a port.
  `MIN_TERMINAL_STUB` must exceed the arrowhead's back-extent (`render`'s
  `markerHeight`, 10) — keep the two in step.

- `decorate::PAGE_INSET = 28.0`, `decorate::BLOCK_GAP = 12.0`, and
  `decorate::BLOCK_PAD = 10.0` — the page-furniture geometry (M21.3). The inset
  is the gap between the canvas edge and the page border rectangle; it exists
  whenever any furniture is declared (so `border` present-vs-absent changes no
  coordinate). `BLOCK_GAP` is the clearance a block keeps from the border, and
  `BLOCK_PAD` the padding inside a text block's frame. The renderer reads
  `PAGE_INSET` (for its palette's corner radius only — the rect itself comes
  from `Layout::border`) and `TB_PAD`/`TB_COL_GAP` (for the title-block grid),
  so the two sides cannot disagree about a table's columns. `MARGIN` (20.0) is
  the *content's* clearance: the border sits outside it, so content keeps its
  pre-M21 guarantee with furniture present.
- `decorate::TEXT_SIZE = 13.0`, `TITLEBLOCK_SIZE = 12.0`, and
  `TITLEBLOCK_TITLE_SIZE = 14.0` — the text-block body size, the title-block
  body size, and the one-size-up size of a `Title` field. The renderer reads
  these to size its `<text>` to the widths `decorate` measured the blocks
  from; a mismatch would overflow the frame.

## Editor tooling

- `contrib/vim/` — vim/neovim runtime files (syntax, ftdetect, ftplugin) for
  `.dgmr` files.
- `contrib/vscode/` — VS Code extension (TextMate grammar) for `.dgmr` files,
  mirroring `contrib/vim/`. The syntax file mirrors `docs/grammar.md` (v0), including
  its contextuality: shapes only after `:`, edge styles only inside an edge
  body, directions only after `diagram`/`group`, text-block anchors only after
  `textblock`. Two classes of reserved word: `diagram`/`group`/`end` are
  **fully** reserved (so a mid-line occurrence is flagged), while
  `border`/`titleblock`/`textblock` are reserved **only at statement start** —
  `a --> border` is a node named `border`, so they must not be flagged
  mid-line. Both `titleblock` and `textblock` are multi-line regions (the only
  ones in the grammar) ending at a line whose trimmed content is exactly
  `end`; the `titleblock` region suppresses the unknown-attribute flag
  (its keys are open by design) and the `textblock` region is left unpainted
  (its body is raw prose: `#` is not a comment, `-->` is not an arrow). If the
  grammar changes — new keywords, shapes, styles, attributes — update
  `contrib/vim/syntax/dgmr.vim` and
  `contrib/vscode/syntaxes/dgmr.tmLanguage.json` alongside the parser and
  `docs/grammar.md`.

## Test layout

- `src/main.rs` — `normalize_source` (BOM/CRLF) unit tests plus an end-to-end
  check that the canonical sample parses identically in all four on-disk
  spellings.
- `src/render/svg.rs` — unit checks + golden snapshots (`assert_snapshot`).
  The M21.3 snapshots (`titleblock`, `textblock`, `page_furniture`) pin the
  decoration; a furniture-free diagram emits no page groups at all.
- `src/layout.rs` — geometry/alignment/cross-boundary tests (the bulk).
  Three `#[ignore]`d inspection helpers dump a layout (`--ignored
  --nocapture`): `_dump_infra_for_inspection` reads `DIAG_SRC` (path to a
  `.dgmr` file) to dump an arbitrary source, `_dump_labels_for_inspection`
  reports edge-label anchor coverage on the samples, and `_trace_m18_stress`
  prints the phase-2 negotiation trace for `DIAG_SRC`. These are the only
  env-driven debug switches; production code has none (M20.6 deleted the one
  that had crept in — a `DIAG_Q` block in `whole_path_candidate`).
- `src/decorate.rs` — M21.3 unit tests: inline-markup parsing (bold/italic/
  nesting, word-boundary `_`, escapes, unbalanced markers literal), block
  layout (wrapping, rules, paragraph gaps), band reservation, and placement
  (a block sits inside the border and clear of the content; the title block
  snaps to the lower-right; a `right` block stacks above it).
- `src/resolve.rs` — validation + error-case tests.
- `src/text.rs` — measurement sanity tests + `font_verification` (proves the
  baked table ≡ live font lookups, exhaustively over all Unicode codepoints;
  the slowest tests in the suite, ~20s).
- `examples/infra.dgmr` and `examples/subdirection.dgmr` are the canonical
  sample diagrams (`examples/` holds `.dgmr` input files only; the
  metrics-table generator lives in the `tools/gen-metrics-table/` workspace
  crate); `infra.dgmr` is loaded by tests via `include_str!`.
  Input files use the `.dgmr` extension (deliberately unique — unclaimed by
  other formats — so editor tooling like a vim syntax file can target the
  grammar unambiguously); don't rename examples back to `.mmd`/other
  extensions without updating the `include_str!` paths in tests.
