---
name: diagrammer
description: Write diagrams in the diagrammer (.dgmr) DSL — a line-oriented language for infrastructure and architecture diagrams with boxes and arrows. It produces a self-contained SVG file that works on GitHub. Use when creating, editing, or validating .dgmr files, or when a user asks for a diagram in "diagrammer format".
---

# Diagrammer DSL

Write diagrams as `.dgmr` text files. Validate each file with the CLI. Repeat the validation until the file produces no errors. Full language specification: `docs/grammar.md` (in the diagrammer repository). Standard examples: `examples/*.dgmr`.

## Validation procedure

```bash
diagrammer path/to/diagram.dgmr              # summary + validation errors (1-based line numbers)
diagrammer path/to/diagram.dgmr -o out.svg   # render SVG
```

Run the command without `-o` after every edit. The output shows validation errors with 1-based line numbers. Fix the errors. Run the command again. Always validate the file before you give it to the user.

## Language reference

A file contains a `diagram <direction>` header, then node and edge statements, then `group ... end` blocks. Write one statement per line. The `#` character starts a comment unless it is inside a string. Indentation does not affect parsing.

```
diagram top-down                    # top-down | bottom-up | left-right | right-left

web "Web Server"                    # node:  id ["Label"] [: shape] [attr*]
db  "Postgres" : cylinder           # shapes: box (default, omittable), cylinder
queue                               # bare id uses the id as its label

web --> lb                          # plain edge
lb  --> api1 --> api2               # chains are fine (single line only)
api1 -- "calls" --> db              # labeled
api1 -- dotted --> cache            # styled: solid | dotted | dashed | thick
db -- thick "repl" color="#888" --> replica "Replica" : cylinder
api -- from="top" to="left" --> db  # force page-space sides (top|bottom|left|right)

group "Kubernetes Cluster"          # group [direction] ["Title"] [attr*]
    api1                            # body = node references / more edges / nested groups
    group left-right "Pods"         # per-group layout direction override
        pod1
    end
end
```

### Attributes

- **Node:** `color` (border, default `#2196F3`), `fill` (background, default `#c0e1fc`), `text` (label color).
- **Edge:** `color` (line + arrowhead), `text` (label color), `from` / `to` (side of the endpoint node: `top|bottom|left|right`).
- **Group:** `color` (frame border), `fill` (frame background; `"none"` keeps it transparent), `line` (frame border style as a *quoted value*: `line="dashed"`), `text` (title color).

Unknown attributes and values cause errors. The tool does not ignore them. For example, `from="north"` and `line="wavy"` cause errors.

### Strings

All strings must be double-quoted. Supported escape sequences: `\"`, `\\`, `\n`. The `\n` escape creates a multi-line label. Nodes, frames, and edge labels all size to the tallest line. No other escape sequences are supported. Inside a string, `#` is a literal character.

### Groups and membership

- A node belongs to a group if the group body mentions it. You can mention a node as a bare id or as an edge endpoint inside the body. Nodes that the body does not mention are top-level nodes.
- You can declare a node in only one group. A cross-group edge endpoint is allowed. The tool routes the edge through the group frames.
- A group can override the layout direction (for example, `group left-right "K8s"`). If you do not specify a direction, the group inherits the direction of the enclosing group.

## Common errors

1. **Quote all color values.** The format `color=#888` causes an error. The `#` character starts a comment and the tool removes the rest of the line. Always use the format `color="#888"`.
2. **Outside a string, `#` starts a comment.** The string `"API Server #1"` is correct. The statement `web --> api1 #1` is incorrect. The tool silently removes everything after `#`.
3. **The edge body must follow this order:** `-- [style] [label] [attrs] -->`.  The format `-- "label" dotted -->` causes an error.
4. **Write one statement per line.** Edge chains cannot continue on the next line. Split long chains into separate statements.
5. **Hyphens in IDs:** An ID can contain `-` (for example, `api-server` is one ID). A hyphen followed by a space or another hyphen is not part of an ID. `A-->B` and `A --> B` produce the same result.
6. **Node redeclaration must be consistent.** If you use a node again in a chain, its label and shape must match the original declaration. A mismatch causes a parse error.
7. **Reserved words:** Only `diagram`, `group`, and `end` are reserved words.  Words such as `box`, `cylinder`, `solid`, and `top-down` are contextual.  You can use them as node names (for example, a node named `box` is valid).
8. **Unsupported features:** No-arrow edges (`---`), style classes, and shapes other than `box` and `cylinder` are not supported. Do not create new syntax. The tool will not parse it.
9. **`from=` and `to=` name the sides of the endpoint node only.** They do not apply to frames. The tool follows these values literally. If a forced side contradicts the flow direction, the edge wraps orthogonally. This is valid but may produce incorrect visual output. If the output looks wrong, remove or change the attribute. Do not expect the layout engine to correct it.

## Style guidance for good output

- Use the default `top-down` layout for architecture diagrams. Use `left-right` groups for large numbers of items, such as pods and node lists.
- Use the cylinder shape for databases, caches, queues, and message stores.
- Keep the number of cross-boundary edges small. Put closely related nodes in the same group. The layout engine routes cross-boundary edges through frame connection points without changing each group's internal layout. Do not force `from=` or `to=` on frame-crossing edges.
- Give every node a readable label. Write IDs in kebab-case (`load-balancer`) or snake_case (`api_1`). Write labels with a capital letter for each word (for example, "Load Balancer").
- Use `dotted` for optional or asynchronous links, `dashed` for weak dependencies, `thick` for the main flow, and `color=` only when necessary to group related edges.
