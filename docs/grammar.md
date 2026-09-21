# Diagram grammar (v0)

This is the concrete spec the parser is built against. It is a small,
line-oriented language inspired by Mermaid's flowchart syntax but not
backward-compatible with it. Goals: readability, simplicity, and room to
grow toward infrastructure diagrams (boxes now, cylinders next).

## Example

```
# diagram.mmd — infrastructure
diagram top-down

web       "Web Server"
lb        "Load Balancer"
api1      "API Server 1"
api2      "API Server 2"
db        "Postgres"   : cylinder
cache     "Redis"      : cylinder
queue     "Message Queue"

web --> lb
lb  --> api1
lb  --> api2

api1 --> db
api2 --> db
api1 -- dotted --> cache
api2 -- dotted --> cache
api1 -- "events" --> queue
api2 -- "events" --> queue

# edge with style + label + color, all between the dashes
db -- thick "replication" color="#888" --> replica "Replica" : cylinder

subgraph "Kubernetes Cluster"
    api1
    api2
end
```

## Grammar

```
diagram     := "diagram" direction EOL statement*
direction   := "top-down" | "bottom-up" | "left-right" | "right-left"
statement   := nodelist | subgraph | (blank)
subgraph    := "subgraph" [direction] [string] [attr*] EOL statement* "end" EOL
nodelist    := nodespec ( edge nodespec )* EOL
nodespec    := id [string] [ ":" shape ] [attr*]
edge        := "-->" | "--" edgebody "-->"
edgebody    := [style] [string] [attr*]
style       := "solid" | "dotted" | "dashed" | "thick"
shape       := "box" | "cylinder"                      # extensible
attr        := id "=" string
id          := [A-Za-z_][A-Za-z0-9_]* ( "-" [A-Za-z0-9_]+ )*
string      := '"' ( [^"\\] | '\' . )* '"'
comment     := '#' ... EOL                              # ignored outside strings
```

## Design principles

1. **Everything between `--` and `-->` belongs to the edge. Everything
   trailing a node's id (its optional label, `: shape`, and attrs) belongs
   to that node.** A modifier attaches to whatever it is syntactically
   inside. This is what keeps the grammar unambiguous as features are
   added.

2. **Line-oriented.** One statement per line. The parser splits on
   newlines and parses each statement independently. Trade-off: edge
   chains can't wrap across lines. If that bites later, add explicit line
   continuation; start strict.

3. **Indentation is not significant.** Subgraph bodies may be indented
   for readability; the parser ignores leading whitespace.

## Tokens

### Identifiers

```
id := [A-Za-z_][A-Za-z0-9_]* ( "-" [A-Za-z0-9_]+ )*
```

A hyphen is part of an id **only if followed by a letter, digit, or
underscore**. So `api-server` is a single id, but in `A-->B` the `--` is
never swallowed — it is always lexed as the edge operator. As a
consequence both `A-->B` and `A --> B` parse identically.

### Strings

Double-quoted, with backslash escapes:

```
string := '"' ( [^"\\] | '\' . )* '"'
```

Recognized escapes: `\"` (literal quote), `\\` (literal backslash), and
`\n` (newline — makes a label or title multi-line; nodes, subgraph frames,
and edge labels all size themselves for the tallest/widest line). Any
other `\X` is a parse error for v0 (we may relax later). Strings are used
for labels, titles, and attribute values. A `#` inside a quoted string is
literal, e.g. `"API Server #1"`.

### Comments

`#` to end of line, ignored everywhere except inside strings.

## Reserved words

Only `diagram`, `subgraph`, and `end` are fully reserved as identifiers.
Everything else (`box`, `cylinder`, `solid`, `dotted`, `dashed`, `thick`,
`top-down`, `left-right`, …) is **contextual**: it only has its special
meaning in a specific syntactic position. `box` only means "shape" after
a `:`, so a node may still be named `box`. This keeps the reserved list
tiny and the language forgiving.

## Nodes

A bare id uses the id as its label. To override, supply a quoted string:

```
web            # label: "web"
web "Web Server"   # label: "Web Server"
```

Shapes follow the label, after a colon. v0 shapes: `box` (default, may be
omitted) and `cylinder`:

```
db "Postgres" : cylinder
cache "Redis" : cylinder
```

Nodes may also be declared **implicitly** inside an edge chain, which keeps
small diagrams compact:

```
web "Web Server" --> lb "Load Balancer" --> api1 "API Server 1"
```

Each node in the chain is created if absent. If a node was declared
earlier, the inline occurrence must be consistent with it (same label or
no label; same shape or no shape). A conflicting redeclaration is a parse
error.

## Edges

A directed edge is `-->` (arrow on the target side). The edge body, when
present, sits between `--` and `-->`:

```
A --> B                      # plain
A -- "label" --> B           # labeled
A -- dotted --> B            # styled
A -- thick "replication" color="#888" --> B   # style + label + color
```

Within the edge body the order is `style label attrs` (each optional).
The parser is strict about this order for v0; it may relax later.

### Edge styles

```
style := "solid" | "dotted" | "dashed" | "thick"
```

`solid` is the default and usually omitted. The other three render as:
`dotted` (fine dots), `dashed` (short dashes), `thick` (heavier solid
stroke).

### No-arrow edges

Deferred. The natural form is `---` (no arrowhead) as the counterpart to
`-->`, but it is out of scope for v0.

## Attributes

```
attr := id "=" string
```

Attributes appear after a node's shape (`db "Postgres" : cylinder color="#888"`)
or within an edge body (`A -- thick color="#888" --> B`). Values are
quoted strings, so they can hold hex colors (`"#888"`, `"#888888"`) or CSS
named colors (`"red"`). Quoting colors is deliberate: a bare `#888` would
be eaten by the `#`-comment lexer.

### Attribute semantics

Parsed in v0 and **rendered as of M6** (node `color`/`fill`, edge `color`
and line styles, and edge labels) and **M7.5** (subgraph `color`/`fill`/
`line`, and a `text` text-color attribute on nodes, edges, and subgraphs).

- **Nodes:**
  - `color` — border / stroke color.
  - `fill` — interior background color.
  - `text` — label text color (M7.5).

- **Edges:**
  - `color` — line and arrowhead color.
  - `text` — edge-label text color (M7.5). Applies only when the edge has
    a label.

- **Subgraphs:**
  - `color` — frame border / stroke color (M7.5).
  - `fill` — frame interior background color (M7.5). The default is
    transparent (`none`), so edges routed behind a subgraph stay visible
    through it.
  - `line` — frame border line style (M7.5): one of `solid`, `dotted`,
    `dashed`, `thick` — the same set as edge styles, but supplied as a
    quoted attribute value (e.g. `line="dashed"`), since the subgraph
    header has no style keyword slot. `thick` doubles the border width;
    `dotted`/`dashed` reuse the edge dash patterns. An unknown value
    (e.g. `line="wavy"`) is a resolve error.
  - `text` — frame title text color (M7.5). Applies only when the subgraph
    has a title.

**Unknown attributes are a parse error.** An attribute is valid only if it
is recognized for the position it appears in (see the per-position lists
above). This catches typos like `colr="#888"` early rather than silently
ignoring them. The recognized attributes are fixed for v0; adding new ones
later is a deliberate grammar change.

## Subgraphs

```
subgraph [direction] [string] [attr*]
    statement*
end
```

In v0 subgraphs are **visual grouping only**. The optional `direction`
slot selects the subgraph's own layout direction (e.g.
`subgraph left-right "K8s"`); a subgraph without one inherits the effective
direction of its enclosing level. The body is a
sequence of statements, typically node references:

```
subgraph "Kubernetes Cluster"
    api1
    api2
end
```

Referencing a node in a subgraph places it in that group; a node not
referenced by any subgraph belongs to the top-level diagram. A node may
not appear in more than one subgraph (parse error).

A subgraph's frame can be styled with `color` (border), `fill`
(background), `line` (border style), and `text` (title color) attributes
(M7.5); see [Attribute semantics](#attribute-semantics). For example:

```
subgraph "Kubernetes Cluster" color="#0a7" fill="#cfe" line="dashed" text="#005"
    api1
    api2
end
```

## Direction

```
direction := "top-down" | "bottom-up" | "left-right" | "right-left"
```

`top-down` ranks root nodes at the top, children below. `left-right`
ranks them left to right. `bottom-up` and `right-left` are the reverses.
The diagram uses a single global direction by default, but any `subgraph`
may override it with its own `direction` (milestone 4); a subgraph without
one inherits the direction of its enclosing level.

## What v0 does not include

- No-arrow edges (`---`).
- Reusable style classes (`classDef`-style).
- Shapes beyond `box` and `cylinder`.
- Cross-boundary edge routing that connects through subgraph frames.
  Frame-aware routing is implemented (M5): an edge crossing a group boundary
  runs to a connection point on the frame (at the frame's center, on the side
  facing the other endpoint along the level's direction axis), then continues
  to the inner node, without disturbing the group's internal layout. The
  segments are routed orthogonally (M7): each is a right-angle path whose jog
  lands in an inter-rank gap or a frame's padding, and a stub crossing a
  titled frame's top side jogs just below the title rather than across it.
- Explicit alignment or placement hints for nodes. Ranks center
  automatically on their parents' centroid (see the layout engine); if that
  proves insufficient for complex diagrams, per-node pinning/alignment
  controls can be added later via the node `attr` slot.

These are all shaped for in the grammar (direction slot in `subgraph`,
`attr` slots on nodes and edges) so adding them later is additive, not a
re-spec.
