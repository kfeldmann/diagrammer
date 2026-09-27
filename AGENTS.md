# AGENTS.md — diagrammer

Read these two docs **first**, before touching code — they tell you where the
project is and what's next without reading source:

- [`docs/milestones.md`](docs/milestones.md) — status + plan (what's done,
  what's next, dependency graph). The **Current position** line at the top
  says which milestone is in flight.
- [`docs/grammar.md`](docs/grammar.md) — the v0 input-language spec the
  parser/lexer are built against.

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
                    cross-boundary frame routing + M9 edge separation &
                    obstacle-aware LCA routing + M13 label placement;
                    forced `from=`/`to=` sides bind the endpoint node only)
  → render/svg.rs   self-contained SVG string
  → main.rs         CLI: `diagrammer <in.dgmr> [-o <out.svg>]`
```

`ast.rs` holds both layers: the `Raw*` syntactic types and the resolved
`Diagram`/`Node`/`Edge`/`Group`. `text.rs` measures labels against the baked
DejaVu Sans metrics in `metrics_table.rs` — generated from the embedded font
by `tools/gen-metrics-table` and verified against it by tests
(deterministic across machines).
`error.rs` is an offset-carrying `Error` enum → 1-based line numbers.

## src/ module map

```
src/
  main.rs        CLI: arg parsing, run(), summary vs. -o SVG output
  lexer.rs       tokenizer
  parser.rs      winnow parser → raw AST
  ast.rs         Raw* + resolved Diagram/Node/Edge/Group + Shape/Style/Direction enums
  resolve.rs     raw → validated Diagram (dedup, attrs, group membership)
  layout.rs      ★ biggest file: flat Sugiyama engine + compound (per-group
                 direction) + cross-boundary edge routing + M9 edge
                 separation & obstacle-aware LCA routing + sealed-port
                 fallback (a lone side's port slides when its escape pocket
                 is sealed) + M13 label placement & label-aware fan/lane
                 spacing + M14 forced sides decoupled from frame crossings
                 → Layout
  text.rs        label measurement against the baked metrics table
  metrics_table.rs  GENERATED baked DejaVu metrics — never hand-edit;
                 regenerate: cargo run -p gen-metrics-table
  error.rs       Error enum + line_of()
  render/
    mod.rs       `pub mod svg;`
    svg.rs       Layout+Diagram → self-contained SVG (boxes, cylinders, edges,
                 edge labels at their layout-given anchors, per-color
                 arrowhead markers)
```

## Build, test, run

```bash
cargo test                            # full suite (incl. golden snapshots)
cargo test -- --skip snapshot         # logic only, faster feedback
UPDATE_SNAPSHOTS=1 cargo test         # regenerate golden .svg files (commit them)
cargo run -p gen-metrics-table       # regenerate src/metrics_table.rs (font changes; commit it)
cargo run -- examples/infra.dgmr -o out.svg   # render one diagram
cargo run -- examples/infra.dgmr               # print a validation summary
```

**The snapshot harness is non-obvious:** `src/render/svg.rs` zips each diagram
against a golden file in `snapshots/`. When rendering output intentionally
changes, regenerate with `UPDATE_SNAPSHOTS=1` and commit the updated `.svg`
files (don't hand-edit them).

**Release builds** are Docker-based and live in `build/`: one script per
platform (`build-linux-glibc`, `build-linux-musl`, `build-mac`), sharing
`build/debian.Dockerfile` (glibc + macOS cross via `cargo-zigbuild`) and
`build/alpine.Dockerfile` (musl). Each mounts the project at `/work` and runs
as the host UID:GID. See `build/README.md`.

## Invariants — don't break these

- **Index correspondence.** `Layout.nodes`/`Layout.edges`/`Layout.groups`
  correspond **by index** to `diagram.nodes`/`diagram.edges`/`diagram.groups`
  in declaration order. The renderer `zip`s them directly. Any new output
  keyed by id must preserve this. (Edge-label anchors ride on `EdgePath` —
  `label_at: Option<(f32, f32)>`, M13 — for exactly this reason; don't move
  them into a separate keyed collection.)
- **Canonical top-down space, then transform.** All layout math runs in
  canonical top-down space (rank grows down, order is horizontal), then a
  rigid transform maps rects + waypoints into the requested direction. The
  same transform applies to node rects and edge waypoints, so ports stay
  glued to the correct side of each box in all four directions. Shape/label
  math that reasons about "up/down" must account for this (e.g. the cylinder
  cap radius and label shift are direction-agnostic because they live in
  rendered box space, not canonical space).
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
- **Deterministic output.** `fmt()` formats floats to exactly 2 decimals; text
  is measured against the baked metrics of the **embedded** DejaVu Sans
  Regular (`src/metrics_table.rs`), never a system font. Snapshots are byte-stable across
  machines — don't introduce nondeterminism (random ids, hashmap iteration
  order leaking into output, system fonts). Edge-label placement (M13) is a
  fixed-order greedy pass plus one refinement pass over candidate anchors —
  keep it that way (no randomness, no iteration-order dependence).
- **Measurement is baked and verified.** `src/metrics_table.rs` is *generated*
  — never hand-edit it; regenerate with `cargo run -p gen-metrics-table`
  (and commit the result) when the bundled font changes. The
  `font_verification` tests in `text.rs` prove the table is bit-identical to
  live `ttf-parser` lookups over the whole Unicode range — keep them green.
  Two load-bearing quirks (pinned by those tests): pixel scaling is
  `font_size / (ascent − descent)` (the em box, **not** `units_per_em`), and
  kerning is *first horizontal subtable wins* with zero-valued pairs treated
  as absent.

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
- `layout::FRAME_TITLE_X = 10.0`, `layout::FRAME_TITLE_TOP = 4.0`, and
  `layout::FRAME_TITLE_FONT_SIZE = 12.0` — the inset, top offset, and font
  size of a group's title text. Layout reads these to detect when a
  cross-boundary within-frame stub would cross the title text
  (`title_detour_clear_x`) so it can route around it, and to keep label
  knockouts off the title (`title_text_rect`, M13); they must match
  `render::FRAME_TITLE_X` (10.0), `render::FRAME_TITLE_Y` (4.0), and
  `render::FRAME_TITLE_SIZE` (12.0), else the title-detour would misjudge the
  title's extent and labels would clear the wrong box.
- `layout::STUB_JOG_CLEARANCE = 12.0` — how far a title-detour within-frame
  stub jogs from a node port (it sits in the grown `CROSS_FRAME_PAD` band just
  below the title). Detours apply per chain frame — one jog per titled frame
  the stub would cross. It must exceed `render::ARROW_BACKOFF`
  (4.0), else the stub's final segment is shorter than the line-end
  shortening and the arrowhead tip pokes into the node; and it must exceed
  the arrowhead's back-extent (`render`'s `markerHeight`, 10.0 — the marker
  base sits `markerHeight` short of the tip), else the horizontal approach
  line cuts into the arrowhead's side. With `CROSS_FRAME_PAD` growing the
  frame it also clears the title text above the node. (One-directional:
  layout does not read the render constants, it just promises to stay above
  them.)
- `layout::CROSS_FRAME_PAD = 12.0` — extra top + bottom padding added to a
  group frame that hosts a cross-boundary within-frame stub's jog: a frame
  with a cross-boundary edge to an immediate child, and — so a nested title
  detour has its band — any *titled* frame in a cross-boundary chain under a
  top-down LCA. The padding gives the stub room to jog (clear of the title
  above the node and of the node's arrowhead below the jog). One group-title
  font height each side; keep in sync with `render::FRAME_TITLE_SIZE` (12.0).
  A group with no such edges keeps the default frame geometry.

## Editor tooling

- `contrib/vim/` — vim/neovim runtime files (syntax, ftdetect, ftplugin) for
  `.dgmr` files. The syntax file mirrors `docs/grammar.md` (v0), including
  its contextuality: shapes only after `:`, edge styles only inside an edge
  body, directions only after `diagram`/`group`, only `diagram`/`group`/`end`
  reserved (and only at statement start). If the grammar changes — new
  keywords, shapes, styles, attributes — update `contrib/vim/syntax/dgmr.vim`
  alongside the parser and `docs/grammar.md`.

## Test layout

- `src/render/svg.rs` — unit checks + golden snapshots (`assert_snapshot`).
- `src/layout.rs` — geometry/alignment/cross-boundary tests (the bulk).
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
