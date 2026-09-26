# diagrammer

A small command-line tool that reads a Mermaid-like diagram DSL and produces
SVG output. It exists to:

- avoid round-tripping through the Mermaid website to preview/tune diagrams,
  and
- give more control over group layout direction than Mermaid (which ignores
  a `subgraph`'s direction when edges cross group boundaries).

## Status

v0: the input language is **parsed, validated, laid out, and rendered to
SVG end to end**, including **compound layout with per-group direction**
(M4), **cross-boundary edge routing through group frames** (M5), and
**shape/style/color rendering** (M6) — groups render as labeled frames,
size to fit their contents, each may declare its own `direction` independent
of the diagram's, and edges crossing a group boundary route to connection
points on the frames without disturbing the groups' internal layout. Nodes
render as rounded boxes or cylinders (with `color`/`fill`); edges render with
the `dotted`/`dashed`/`thick` styles and per-edge `color`, and edge labels
sit at the edge midpoint. Orthogonal edge routing (M7) is the next milestone
(see the plan in `docs/milestones.md`).

```console
$ cargo run -- examples/infra.dgmr
ok: 8 nodes, 10 edges, 1 groups (direction top-down)

$ cargo run -- examples/infra.dgmr -o examples/infra.svg
wrote examples/infra.svg
```

The written SVG is self-contained and GitHub-renderable (no scripts or
external references).

## Usage

```
diagrammer <input.dgmr> [-o <output.svg>]
```

Input files use the `.dgmr` extension — a deliberately unique extension
(unclaimed by other formats) so editor tooling, like a vim syntax file, can
target diagrammer's grammar unambiguously. A vim syntax file matching the v0
grammar ships in [`contrib/vim`](contrib/vim).

Without `-o`, `diagrammer` parses and validates the input and prints a
one-line summary. With `-o <output.svg>`, it lays the diagram out and writes a
self-contained SVG file. Run `diagrammer --help` for details.

## Input language

See [`docs/grammar.md`](docs/grammar.md) for the full grammar. Short example:

```
diagram top-down

web "Web Server" --> lb "Load Balancer"
lb --> api1
lb --> api2

api1 -- dotted --> cache
db "Postgres" : cylinder

group "Kubernetes Cluster"
    api1
    api2
end
```

See [`examples/subdirection.dgmr`](examples/subdirection.dgmr) for a diagram
that puts two groups with *different* directions in one diagram.

Highlights:

- `diagram top-down | bottom-up | left-right | right-left`.
- `group [direction] "title" ... end` — visual grouping; the optional
  direction lays the group out independently of the diagram's direction.
- Nodes: `id` (label defaults to the id) or `id "Label"`, optional `: shape`
  (`box`, `cylinder`), and attributes (`color="..."`, `fill="..."`).
- Edges: `-->`, or `-- style "label" attr=... -->` for a styled/labeled edge
  (`solid | dotted | dashed | thick`).
- `#` line comments. Double-quoted strings with `\"` / `\\` escapes.
- Implicit node declarations in edge chains are allowed; conflicting
  redeclarations are an error. Unknown attributes are an error.
