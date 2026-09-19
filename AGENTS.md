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
per-subgraph layout direction when edges cross group boundaries** — diagrammer
escapes that by laying each subgraph under its own direction and routing
cross-boundary edges through frame connection points without disturbing the
groups' internal arrangement.

## Pipeline (end to end)

```
source string
  → lexer.rs        tokens (ids w/ hyphen rule, quoted strings, # comments)
  → parser.rs       raw AST (RawDiagram: every decl as written, incl. dups)
  → resolve.rs      validated Diagram (dedup nodes, strict attrs, subgraph
                    membership + containment tree, per-subgraph direction)
  → layout.rs       Layout (Sugiyama flat engine + compound recursion +
                    cross-boundary frame routing)
  → render/svg.rs   self-contained SVG string
  → main.rs         CLI: `diagrammer <in.mmd> [-o <out.svg>]`
```

`ast.rs` holds both layers: the `Raw*` syntactic types and the resolved
`Diagram`/`Node`/`Edge`/`Subgraph`. `text.rs` measures labels with `ab_glyph`
against the embedded DejaVu Sans Regular (deterministic across machines).
`error.rs` is an offset-carrying `Error` enum → 1-based line numbers.

## src/ module map

```
src/
  main.rs        CLI: arg parsing, run(), summary vs. -o SVG output
  lexer.rs       tokenizer
  parser.rs      winnow parser → raw AST
  ast.rs         Raw* + resolved Diagram/Node/Edge/Subgraph + Shape/Style/Direction enums
  resolve.rs     raw → validated Diagram (dedup, attrs, subgraph membership)
  layout.rs      ★ biggest file: flat Sugiyama engine + compound (per-subgraph
                 direction) + cross-boundary edge routing → Layout
  text.rs        ab_glyph label measurement vs embedded DejaVu Sans
  error.rs       Error enum + line_of()
  render/
    mod.rs       `pub mod svg;`
    svg.rs       Layout+Diagram → self-contained SVG (boxes, cylinders, edges,
                 edge labels, per-color arrowhead markers)
```

## Build, test, run

```bash
cargo test                            # full suite (incl. golden snapshots)
cargo test -- --skip snapshot         # logic only, faster feedback
UPDATE_SNAPSHOTS=1 cargo test         # regenerate golden .svg files (commit them)
cargo run -- examples/infra.mmd -o out.svg   # render one diagram
cargo run -- examples/infra.mmd                # print a validation summary
```

**The snapshot harness is non-obvious:** `src/render/svg.rs` zips each diagram
against a golden file in `snapshots/`. When rendering output intentionally
changes, regenerate with `UPDATE_SNAPSHOTS=1` and commit the updated `.svg`
files (don't hand-edit them).

## Invariants — don't break these

- **Index correspondence.** `Layout.nodes`/`Layout.edges`/`Layout.subgraphs`
  correspond **by index** to `diagram.nodes`/`diagram.edges`/`diagram.subgraphs`
  in declaration order. The renderer `zip`s them directly. Any new output
  keyed by id must preserve this.
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
  marker.
- **Deterministic output.** `fmt()` formats floats to exactly 2 decimals; text
  is measured against the **embedded** DejaVu Sans Regular (via the `dejavu`
  crate + `ab_glyph`), never a system font. Snapshots are byte-stable across
  machines — don't introduce nondeterminism (random ids, hashmap iteration
  order leaking into output, system fonts).

## Shared constants across the layout/render boundary

These two are defined in `layout.rs` and read by `render/svg.rs` — keep them
in sync (changing one without the other silently breaks geometry):

- `layout::FONT_SIZE = 14.0` — node label size; the renderer sizes `<text>`
  to match the boxes laid out from these metrics.
- `layout::CYL_RY = 5.0` — cylinder elliptical-cap radius; layout sizes the
  cylinder box to include it (`cyl_height`), the renderer draws the caps at
  this radius.

## Test layout

- `src/render/svg.rs` — unit checks + golden snapshots (`assert_snapshot`).
- `src/layout.rs` — geometry/alignment/cross-boundary tests (the bulk).
- `src/resolve.rs` — validation + error-case tests.
- `src/text.rs` — measurement sanity tests.
- `examples/infra.mmd` and `examples/subdirection.mmd` are the canonical
  sample diagrams; `infra.mmd` is loaded by tests via `include_str!`.
