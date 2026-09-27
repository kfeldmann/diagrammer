---
name: diagrammer
description: Write diagrams in the diagrammer (.dgmr) DSL — a Mermaid-like line-oriented language for boxes-and-arrows infrastructure/architecture diagrams that renders as self-contained GitHub-ready SVG. Use when creating, editing, or validating .dgmr files, or when a user asks for a diagram in "diagrammer format".
---

# Diagrammer DSL

Write diagrams as `.dgmr` text, validate with the CLI, iterate until it
renders clean. Full language spec: `docs/grammar.md` (in the diagrammer
repo). Canonical samples: `examples/*.dgmr`.

## The one validation loop that matters

```bash
diagrammer path/to/diagram.dgmr              # summary + validation errors (1-based line numbers)
diagrammer path/to/diagram.dgmr -o out.svg   # render SVG
```

Run the no-`-o` form after every edit. Errors carry line numbers; fix and
re-run. Never hand the user a `.dgmr` file that you have not validated this
way.

## Language in one page

A file is: `diagram <direction>` header, then node/edge statements, then
`group ... end` blocks. One statement per line. `#` starts a comment
(unless inside a string). Indentation is free.

```
diagram top-down          # top-down | bottom-up | left-right | right-left

web "Web Server"          # node:  id ["Label"] [: shape] [attr*]
db  "Postgres" : cylinder # shapes: box (default, omittable), cylinder
queue                     # bare id uses the id as its label

web --> lb                # plain edge
lb  --> api1 --> api2     # chains are fine (single line only)
api1 -- "calls" --> db    # labeled
api1 -- dotted --> cache  # styled: solid | dotted | dashed | thick
db -- thick "repl" color="#888" --> replica "Replica" : cylinder
api -- from="top" to="left" --> db   # force page-space sides (top|bottom|left|right)

group "Kubernetes Cluster"          # group [direction] ["Title"] [attr*]
    api1                            # body = node references / more edges / nested groups
    group left-right "Pods"         # per-group layout direction override
        pod1
    end
end
```

### Attributes

- **Node:** `color` (border, default `#2196F3`), `fill` (background, default
  `#c0e1fc`), `text` (label color).
- **Edge:** `color` (line + arrowhead), `text` (label color), `from` / `to`
  (side of the endpoint node: `top|bottom|left|right`).
- **Group:** `color` (frame border), `fill` (frame background; `"none"`
  keeps it transparent), `line` (frame border style as a *quoted value*:
  `line="dashed"`), `text` (title color).

Unknown attributes/values are **errors** (no silent ignore) — e.g. `from="north"`
or `line="wavy"` fail to resolve.

### Strings

Double-quoted. Escapes: `\"`, `\\`, `\n` (multi-line labels; nodes, frames,
and edge labels all size for the tallest line). Nothing else. A `#` inside
quotes is literal.

### Groups and membership

- A node belongs to a group if a group body mentions it (bare id, or as an
  edge endpoint in the body). Unmentioned nodes are top-level.
- A node may be *declared* in only one group; a cross-group edge endpoint is
  fine — the edge just crosses the boundary (routed through the frames).
- A group may override direction (`group left-right "K8s"`); it inherits the
  enclosing direction otherwise.

## Pitfalls that cause real errors

1. **Colors must be quoted.** Bare `color=#888` fails (the `#` starts a
   comment and the rest of the line vanishes). Always `color="#888"`.
2. **`#` inside an unquoted context is a comment.** `"API Server #1"` is
   fine; `web --> api1 #1` silently truncates the line.
3. **Edge-body order is strict:** `-- [style] [label] [attrs] -->`.
   `-- "label" dotted -->` is an error.
4. **One statement per line.** Edge chains cannot wrap; split long chains
   into separate statements.
5. **Hyphen rule:** ids may contain `-` (`api-server` is one id), but a
   hyphen followed by space/dash is not part of an id. `A-->B` and `A --> B`
   are identical.
6. **Implicit redeclaration must be consistent.** Reusing a node in a chain
   with a different label or shape than its declaration is a parse error.
7. **Only `diagram`, `group`, `end` are reserved.** `box`, `cylinder`,
   `solid`, `top-down`, etc. are contextual — a node may be named `box`.
8. **No-arrow edges (`---`), style classes, and shapes beyond box/cylinder
   don't exist yet.** Don't invent syntax; it will not parse.
9. **`from=`/`to=` name sides of the endpoint node only**, never frames, and
   are honored literally — a forced side contradicting the flow direction
   wraps orthogonally (valid but ugly). If output looks bad, drop or change
   the attribute rather than expecting the layout to second-guess it.

## Style guidance for good output

- Prefer the default `top-down` for architecture diagrams; use
  `left-right` groups for wide swaths (pods, node lists).
- Cylinders for databases/caches/queues/messages — that's what they're for.
- Keep cross-boundary edges few; put tightly-coupled nodes in one group.
  The engine routes cross-boundary edges through frame connection points
  without disturbing each group's internal layout — don't fight it by
  forcing `from=`/`to=` on frame-crossing edges.
- Give every node a human label; ids stay kebab/snake (`load-balancer`,
  `api_1`), labels are Title Case ("Load Balancer").
- Use `dotted` for optional/async links, `dashed` for soft dependencies,
  `thick` for the main flow, and `color=` sparingly to group related edges.
