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

A diagram may also carry page furniture — a border, a title block, and text
blocks — none of which participate in layout or routing (see
[Page furniture](#page-furniture)):

```
diagram top-down

border color="#333"

titleblock
    Title   = "Finance: Network Architecture"
    Company = "Big Corp, LLC"
    Version = "0.1"
end

textblock right
    Here is some text with _limited_ **Markdown** support.
    Each line is a paragraph.
    ---
    Horizontal rules are supported.
end
```

## Grammar

```
diagram     := "diagram" direction EOL statement*
direction   := "top-down" | "bottom-up" | "left-right" | "right-left"
statement   := nodelist | group | border | titleblock | textblock | (blank)
group       := "group" [direction] [string] [attr*] EOL (nodelist | group | (blank))* "end" EOL
border      := "border" attr* EOL
titleblock  := "titleblock" attr* EOL field* "end" EOL
field       := attr+ EOL
textblock   := "textblock" anchor attr* EOL rawline* "end" EOL
anchor      := "top" | "upper-right" | "right" | "lower-right"
             | "bottom" | "lower-left" | "left" | "upper-left"
rawline     := <literal source line> EOL                # not tokenized
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

4. **A block's body uses the rules its content needs.** Three blocks share
   one frame (`keyword [header] … end`), but their bodies differ because
   their content does: a `group` body is statements, a `titleblock` body is
   key/value fields (the ordinary `attr` rule), and a `textblock` body is
   raw text. Forcing one body syntax on all three is what would create
   ambiguity — a title-block field written as a bare label is
   lexically a node declaration, and prose tokenized as statements turns
   `#` and `end` into landmines.

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

`#` to end of line, ignored everywhere except inside strings — and inside a
**text block body**, which is raw text (see [Text block](#text-block)), so
`#` there is literal prose.

### File encoding and line endings

Source files are **UTF-8**. A leading UTF-8 BOM (`EF BB BF`) is accepted and
stripped, and both `LF` and `CRLF` line endings work — so a `.dgmr` written by
Notepad or PowerShell parses the same as one written on Linux. A UTF-16 file is
rejected as invalid UTF-8. Normalization happens once, when the file is read,
not in the lexer; a lone `CR` is *not* a line ending.

This matters most for a **text block body**, which is copied verbatim into the
drawing: normalization is what keeps a Windows-authored block free of stray
`CR` characters in the rendered text (and in its measured width).

## Reserved words

Only `diagram`, `group`, `end`, `border`, `titleblock`, and `textblock` are
fully reserved as identifiers. Everything else (`box`, `cylinder`, `solid`,
`dotted`, `dashed`, `thick`, `top-down`, `left-right`, …) is **contextual**:
it only has its special meaning in a specific syntactic position. `box` only
means "shape" after a `:`, so a node may still be named `box`. This keeps the
reserved list tiny and the language forgiving.

The four page-furniture keywords are reserved **only at statement start**
(the same position rule `group` and `end` already follow), because that is
where the parser must choose between "a node declaration" and "a block". A
node may still be named `border` or `textblock` as an edge endpoint
(`a --> border` is a node), and an anchor word such as `right` or `top` is
contextual — it only means "anchor" in the slot after `textblock`:

```
border                # the page border statement, not a node
a --> border          # a node named `border`
textblock right       # a text block anchored right
right --> left        # nodes named `right` and `left`
```

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
    An **empty string** (`color=""`) draws no border at all.
  - `fill` — frame interior background color (M7.5). Defaults to
    `"#f2f8f4"`; an explicit `fill="none"` keeps the frame transparent so
    edges routed behind a group stay visible through it. An **empty string**
    (`fill=""`) is equivalent to `fill="none"`.
  - `line` — frame border line style (M7.5): one of `solid`, `dotted`,
    `dashed`, `thick` — the same set as edge styles, but supplied as a
    quoted attribute value (e.g. `line="dashed"`), since the group
    header has no style keyword slot. `thick` doubles the border width;
    `dotted`/`dashed` reuse the edge dash patterns. An unknown value
    (e.g. `line="wavy"`) is a resolve error.
  - `text` — frame title text color (M7.5). Applies only when the group
    has a title.

  A group with both `color=""` and `fill=""` renders **fully invisible** —
  no border, no interior — which is useful when a group exists only to
  control the layout of its contents (the group still behaves normally
  otherwise: geometry, layout direction, and title placement are
  unchanged).

- **Page furniture** (see [Page furniture](#page-furniture)):
  - `border`: `color` (stroke; `""` draws nothing) and `line` (a quoted
    border style, same set as a group's).
  - `titleblock`: `color` and `fill` (table stroke and interior); every other
    key is a displayed field, rendered verbatim in declaration order.
  - `textblock`: `color` (border), `fill` (interior, default `"seashell"`),
    `text` (text color, default `"sandybrown"`), and `width` (a maximum
    content width in pixels, for word wrapping).

**Unknown attributes are a parse error.** An attribute is valid only if it
is recognized for the position it appears in (see the per-position lists
above). This catches typos like `colr="#888"` early rather than silently
ignoring them. The recognized attributes are fixed for v0; adding new ones
later is a deliberate grammar change.

The single exception is a **title block field key**, which is open by design
(see [Title block](#title-block)) — a title block is a metadata table whose
keys the user chooses, so an unrecognized key is data, not a typo.

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

## Page furniture

Three statement types draw on the page *around* the diagram instead of in it:
`border`, `titleblock`, and `textblock`. They are **decoration only**. None
of them is a node, an edge endpoint, or a group member; they never join the
layout graph, never appear in `Layout::nodes` / `Layout::edges` /
`Layout::groups`, and never act as an obstacle for routing. They are placed
after routing, against the finished canvas.

All three are **optional and independent**: any combination is legal, and a
diagram that uses none of them renders exactly as it did before they
existed.

None of the three may appear inside a `group` body — a block there is a
parse error (`titleblock` and `textblock` cannot be group members; `border`
belongs to the page, not to a level).

### The page inset

The page reserves a fixed **inset** (28 px) between the canvas edge and the
border. It exists whether or not a `border` statement is present: the
border, the title block, and the text blocks are all positioned against it,
so adding or removing `border` changes *no coordinate* — it only strokes the
rectangle that was already reserved.

Diagram content keeps its own guarantee unchanged: every node, frame, edge
waypoint, and label knockout stays at least the page margin (20 px) clear of
the canvas edge, and the border sits outside that margin.

### Border

```
border [attr*]
```

At most one per diagram. `color` (stroke) and `line` (`solid`, `dotted`,
`dashed`, `thick` — a quoted value, like a group's `line`) are supported;
`color=""` draws no border. With no `border` statement nothing is stroked,
but the inset is still reserved (so the title block and text blocks sit
where they would anyway).

The border is drawn *outside* the page margin, so it never crosses a node, a
frame, or an edge — which is why `border` is purely decoration and can be
added or removed without touching layout.

### Title block

```
titleblock [attr*]
    field*
end
```

At most one per diagram. It draws a table in the **lower-right corner**, and
its bottom and right edges snap to the page inset rectangle — the same
rectangle `border` strokes, whether or not a `border` statement is present.
Its body is a list of fields, one or more per line, written with the
language's ordinary attribute syntax:

```
titleblock
    Title   = "Finance: Network Architecture"
    Company = "Big Corp, LLC"
    Author  = "John Doe"
    Version = "0.1"
    Date    = "2026-11-04"
end
```

Keys are **verbatim and open**: a title block is a metadata table, so any key
is accepted and rendered exactly as written, in declaration order (`Version`,
not `version` — the user owns capitalization, so an acronym like
`API Gateway` survives). This is a deliberate, documented exception to the
"unknown attribute is an error" rule: a mistyped key shows up as a mistyped
label in the drawing, which is self-evident, whereas a silently ignored
attribute would not be.

`color` and `fill` are the two **style** keys (they are not fields), and a
`Title` field is drawn one size larger than the rest — that single piece of
hierarchy is what makes the table read as a title block rather than a list.
A duplicate key is a resolve error.

Fields may also be written on the header line, since header and body use the
same rule — `titleblock Title="X" Company="Y"` followed by `end` is a
complete, legal block.

### Text block

```
textblock <anchor> [attr*]
    rawline*
end
```

Zero or more per diagram, each at a distinct anchor. The anchor is one of the
eight positions around the content:

```
upper-left    top        upper-right
left                     right
lower-left    bottom     lower-right
```

Anchors are **page-space** — `top` always means the visual top, whatever the
diagram's direction or any group's — and they mirror the vocabulary of the
`from=` / `to=` edge sides, extended with the four corners. A second block at
the same anchor is a resolve error (stacking at one anchor is deliberately
not in this version).

Attributes: `color` (border), `fill` (interior, default `"seashell"`),
`text` (text color, default `"sandybrown"`), and `width` (a maximum content
width in pixels; the body is word-wrapped to it).

The body is **raw**: each line is taken literally up to the newline. In
particular `#` is *not* a comment inside a block body (unlike everywhere else
in the language), so prose containing `#`, `-->`, or anything else that looks
like syntax is safe.

The block ends at a line whose trimmed content is exactly `end`. Because
`end` is a reserved word, a statement consisting of `end` is unwritable
anyway, so the rule is consistent with the rest of the language rather than a
new special case. (A line of prose that is only the word `end` is therefore
not expressible; the same is true of a bare `end` statement today.)

Within the body a small subset of Markdown is understood, applied **per
line** (an inline run never spans a line break):

| Markup | Meaning |
| --- | --- |
| `**text**` | bold |
| `_text_` or `*text*` | italic |
| `**bold _and italic_**` | nesting works — a run carries both |
| `---` alone on a line | horizontal rule |
| blank line | paragraph gap |

Rules that make prose behave:

- Leading indentation and trailing whitespace are stripped from every line.
- `_` and `*` open a run only at a word boundary, so `snake_case_name` is
  literal text. An underscore in the middle of a word never italicizes.
- `\*`, `\_`, and `\\` are escapes, so literal markers are always writable.
- An **unbalanced** marker is literal text, not an error (`a ** b` renders as
  written). Prose is not code; the language is strict about syntax and
  forgiving about typography.
- An empty body is a resolve error.

```
textblock right
    Here is some text with _limited_ **Markdown** support.
    Each line is a paragraph.

    A blank line above separates paragraphs.
    ---
    Horizontal rules are supported, as is a run that is **bold _and italic_**.
end
```

Text blocks never overlap diagram content: each occupied side reserves a band
between the content and the border, and the canvas grows if the content would
otherwise be covered. A corner anchor takes its space from both of its sides.

The title block is reserved first, since it owns the lower-right corner of the
page. A `textblock lower-right` therefore conflicts with a `titleblock` (that
is a resolve error), and a `right` or `bottom` block occupies the span that
remains beside the title block rather than the full side.

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
- **Page furniture that participates in the diagram.** A `textblock` cannot
  be an edge endpoint, cannot be a group member, and is never a routing
  obstacle; a `titleblock` is decoration only. Two text blocks cannot share
  an anchor (stacking at one anchor is deferred). Markdown in a text block is
  a closed subset: no headings, lists, links, code spans, or block quotes,
  and an inline run never spans a line break. Text blocks do not wrap unless
  `width=` is given.
- **A border per group, or any border other than the page's.** `border` is a
  page-level statement; group frames remain the only nested frames.

These are all shaped for in the grammar (direction slot in `group`,
`attr` slots on nodes and edges) so adding them later is additive, not a
re-spec.
