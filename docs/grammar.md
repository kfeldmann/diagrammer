# Diagram grammar (v0)

This is the concrete spec the parser is built against. It is a small,
line-oriented language inspired by Mermaid's flowchart syntax but not
backward-compatible with it. Goals: readability, simplicity, and room to
grow toward infrastructure diagrams (boxes now, cylinders next).

## Example

```
# diagram.dgmr — infrastructure
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

group "Kubernetes Cluster"
    api1
    api2
end
```

## Grammar

```
diagram     := "diagram" direction EOL statement*
direction   := "top-down" | "bottom-up" | "left-right" | "right-left"
statement   := nodelist | group | (blank)
group       := "group" [direction] [string] [attr*] EOL statement* "end" EOL
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

3. **Indentation is not significant.** Group bodies may be indented
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
`\n` (newline — makes a label or title multi-line; nodes, group frames,
and edge labels all size themselves for the tallest/widest line). Any
other `\X` is a parse error for v0 (we may relax later). Strings are used
for labels, titles, and attribute values. A `#` inside a quoted string is
literal, e.g. `"API Server #1"`.

### Comments

`#` to end of line, ignored everywhere except inside strings.

## Reserved words

Only `diagram`, `group`, and `end` are fully reserved as identifiers.
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

### Edge sides (M11)

An edge may request which side of each endpoint it connects to, via the
`from` and `to` edge-body attributes:

```
A -- from="right" to="top" --> B      # leave A's right side, enter B's top
api -- from="top" --> db              # only the source side is forced
```

Each value is one of `top | bottom | left | right`. Sides are
**page-space** (as drawn): `top` always means the visual top of the box,
regardless of the diagram's or any group's layout direction — no
translation between the attribute and the drawn result.

The request is honored **literally**: the port lands on the requested side,
with no automatic reversion to the layout algorithm's implicit choice.
Silently second-guessing the attribute would leave the user unable to tell
whether it did anything. When a forced side contradicts the approach
direction (e.g. an edge flowing downward forced to leave the source's
top side), the route is wrapped around the node instead. The result may
be ugly — orthogonal detours, wide excursions — but stays valid: segments
remain orthogonal, no route passes through a node interior, no two edges
lie collinearly on top of each other, and several edges forced onto the
same side of a node are fanned apart. An ugly route is the user's cue to
change or drop the attribute.

The attribute names the side of the **endpoint node** only (M14). The
group frames an edge crosses between the endpoints keep the layout's
natural connection points — the side facing the other endpoint — so a
forced side never drags a frame crossing around a group. When the forced
side faces away from where the edge arrives, the within-frame stub bends
instead: it descends beside the node in a clear corridor and hooks into
the forced side (e.g. `to="right"` under top-down enters the group from
above and turns left into the node's right side).

Unknown side values (e.g. `from="north"`) are resolve errors, like any
unknown attribute value.

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
and line styles, and edge labels), **M7.5** (group `color`/`fill`/
`line`, and a `text` text-color attribute on nodes, edges, and groups),
and **M11** (edge `from`/`to` side attributes; see
[Edge sides](#edge-sides)).

- **Nodes:**
  - `color` — border / stroke color. Defaults to `"#2196F3"`.
  - `fill` — interior background color. Defaults to `"#c0e1fc"`.
  - `text` — label text color (M7.5).

- **Edges:**
  - `color` — line and arrowhead color.
  - `text` — edge-label text color (M7.5). Applies only when the edge has
    a label.
  - `from` / `to` — the page-space side of the source / target **node** the
    edge connects to (M11; the node only, never the frames it sits in —
    M14); see [Edge sides](#edge-sides).

- **Groups:**
  - `color` — frame border / stroke color (M7.5). Defaults to `"#88BDA4"`.
  - `fill` — frame interior background color (M7.5). Defaults to
    `"#f2f8f4"`; an explicit `fill="none"` keeps the frame transparent so
    edges routed behind a group stay visible through it.
  - `line` — frame border line style (M7.5): one of `solid`, `dotted`,
    `dashed`, `thick` — the same set as edge styles, but supplied as a
    quoted attribute value (e.g. `line="dashed"`), since the group
    header has no style keyword slot. `thick` doubles the border width;
    `dotted`/`dashed` reuse the edge dash patterns. An unknown value
    (e.g. `line="wavy"`) is a resolve error.
  - `text` — frame title text color (M7.5). Applies only when the group
    has a title.

**Unknown attributes are a parse error.** An attribute is valid only if it
is recognized for the position it appears in (see the per-position lists
above). This catches typos like `colr="#888"` early rather than silently
ignoring them. The recognized attributes are fixed for v0; adding new ones
later is a deliberate grammar change.

## Groups

```
group [direction] [string] [attr*]
    statement*
end
```

In v0 groups are **visual grouping only**. The optional `direction`
slot selects the group's own layout direction (e.g.
`group left-right "K8s"`); a group without one inherits the effective
direction of its enclosing level. The body is a
sequence of statements, typically node references:

```
group "Kubernetes Cluster"
    api1
    api2
end
```

Referencing a node in a group places it in that group; a node not
referenced by any group belongs to the top-level diagram. Both forms of
reference count — a bare declaration and an edge-chain endpoint alike
(`bus --> queue` inside a group declares `bus` and `queue` as members).
A node may not be *declared* in more than one group (parse error); an
edge endpoint naming a node that already belongs to a different group is
a cross-boundary reference instead — the node keeps its group and the
edge runs across the boundary (this is how a chain in an outer body
targets a nested group's member). Occurrences outside any group never
move a node, so membership is decided by group bodies alone and
declaration order cannot silently un-group a node.

A group's frame can be styled with `color` (border), `fill`
(background), `line` (border style), and `text` (title color) attributes
(M7.5); see [Attribute semantics](#attribute-semantics). For example:

```
group "Kubernetes Cluster" color="#0a7" fill="#cfe" line="dashed" text="#005"
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
The diagram uses a single global direction by default, but any `group`
may override it with its own `direction` (milestone 4); a group without
one inherits the direction of its enclosing level.

## What v0 does not include

- No-arrow edges (`---`).
- Reusable style classes (`classDef`-style).
- Shapes beyond `box` and `cylinder`.
- Cross-boundary edge routing that connects through group frames.
  Frame-aware routing is implemented (M5): an edge crossing a group boundary
  runs to a connection point on the frame (on the side facing the other
  endpoint along the level's direction axis, at the endpoint node's
  cross-coordinate), then continues
  to the inner node, without disturbing the group's internal layout. The
  segments are routed orthogonally (M7): each is a right-angle path whose jog
  lands in an inter-rank gap or a frame's padding, and a stub crossing a
  titled frame's top side jogs just below the title rather than across it.
  A forced endpoint side (`from=` / `to=`) bends the inner stub only (M14);
  the frame crossings stay on their natural sides.
- Explicit alignment or placement hints for nodes. Ranks center
  automatically on their parents' centroid (see the layout engine); if that
  proves insufficient for complex diagrams, per-node pinning/alignment
  controls can be added later via the node `attr` slot.

These are all shaped for in the grammar (direction slot in `group`,
`attr` slots on nodes and edges) so adding them later is additive, not a
re-spec.
