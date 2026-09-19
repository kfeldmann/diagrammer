# diagrammer

A small command-line tool that reads a Mermaid-like diagram DSL and produces
SVG output. It exists to:

- avoid round-tripping through the Mermaid website to preview/tune diagrams,
  and
- give more control over subgraph layout direction than Mermaid (which ignores
  per-subgraph direction when edges cross group boundaries).

## Status

v0: the input language is **parsed, validated, laid out, and rendered to
SVG end to end**, including **compound layout with per-subgraph direction**
(M4) and **cross-boundary edge routing through subgraph frames** (M5) —
subgraphs render as labeled frames, size to fit their contents, each may
declare its own `direction` independent of the diagram's, and edges crossing a
group boundary route to connection points on the frames without disturbing
the groups' internal layout. Shape/style/color rendering (M6) is the next
milestone (see the plan in `docs/milestones.md`).

```console
$ cargo run -- examples/infra.mmd
ok: 8 nodes, 10 edges, 1 subgraphs (direction top-down)

$ cargo run -- examples/infra.mmd -o examples/infra.svg
wrote examples/infra.svg
```

The written SVG is self-contained and GitHub-renderable (no scripts or
external references).

## Usage

```
diagrammer <input.mmd> [-o <output.svg>]
```

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

subgraph "Kubernetes Cluster"
    api1
    api2
end
```

See [`examples/subdirection.mmd`](examples/subdirection.mmd) for a diagram
that puts two subgraphs with *different* directions in one diagram.

Highlights:

- `diagram top-down | bottom-up | left-right | right-left`.
- `subgraph [direction] "title" ... end` — visual grouping; the optional
  direction lays the group out independently of the diagram's direction.
- Nodes: `id` (label defaults to the id) or `id "Label"`, optional `: shape`
  (`box`, `cylinder`), and attributes (`color="..."`, `fill="..."`).
- Edges: `-->`, or `-- style "label" attr=... -->` for a styled/labeled edge
  (`solid | dotted | dashed | thick`).
- `#` line comments. Double-quoted strings with `\"` / `\\` escapes.
- Implicit node declarations in edge chains are allowed; conflicting
  redeclarations are an error. Unknown attributes are an error.
