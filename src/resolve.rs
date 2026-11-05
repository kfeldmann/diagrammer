//! Semantic validation: turns a raw syntactic [`RawDiagram`] into a validated
//! [`Diagram`].
//!
//! Responsibilities:
//! - Deduplicate node declarations, enforcing label / shape / attribute
//!   consistency across redeclarations.
//! - Assign group membership. A node may be placed in at most one group;
//!   placing it in two is an error. Any reference inside a `group` body —
//!   a standalone declaration or an edge-chain endpoint alike — moves the
//!   node into that group (declaring at top level and referencing inside a
//!   `group` body is the common pattern). Occurrences at top level never
//!   move a node, and an edge endpoint naming a node already placed in a
//!   *different* group is a cross-boundary reference that leaves it put.
//! - Validate attributes strictly: unknown attributes and duplicate
//!   attributes within a single declaration are errors. As of M11 the
//!   recognized attributes are: nodes `color`/`fill`/`text`, edges
//!   `color`/`text`/`from`/`to` (page-space side requests, validated
//!   against the [`EdgeSide`] set), and groups `color`/`fill`/`line`/
//!   `text` (where `line` is a `solid`/`dotted`/`dashed`/`thick` border
//!   style, validated against the [`Style`] set).
//! - Record the group containment tree (each group's `parent` and
//!   `children`) and carry each group's optional per-group `direction`
//!   through to layout (M4).

use std::collections::HashMap;

use crate::ast::*;
use crate::error::Error;

struct NodeInfo {
    id: String,
    label: Option<String>,
    shape: Option<Shape>,
    color: Option<String>,
    fill: Option<String>,
    text: Option<String>,
    /// Current group: `None` = top level, `Some(idx)` = a group.
    membership: Option<usize>,
    /// Distinct groups this node has been placed in. Length >= 2 is an
    /// error ("appears in more than one group").
    placed_groups: Vec<usize>,
}

struct GroupInfo {
    title: Option<String>,
    direction: Option<Direction>,
    color: Option<String>,
    fill: Option<String>,
    line: Option<Style>,
    text: Option<String>,
    /// Enclosing group index; `None` = top level. Captured when the
    /// group is opened so the resolved model can express nesting.
    parent: Option<usize>,
}

struct Ctx {
    nodes: Vec<NodeInfo>,
    id_index: HashMap<String, usize>,
    edges: Vec<Edge>,
    groups: Vec<GroupInfo>,
    /// Page furniture (M21.1). At most one of each; see
    /// [`Ctx::process_border`] / [`Ctx::process_title_block`].
    border: Option<Border>,
    title_block: Option<TitleBlock>,
    /// Each text block with the offset of its `textblock` keyword. The offset
    /// rides along because one check — the lower-right / title-block conflict
    /// — can only be decided once the whole diagram is read (either statement
    /// may come first), and it still has to point at the text block.
    text_blocks: Vec<(TextBlock, usize)>,
}

pub fn resolve(raw: &RawDiagram) -> Result<Diagram, Error> {
    let mut ctx = Ctx {
        nodes: Vec::new(),
        id_index: HashMap::new(),
        edges: Vec::new(),
        groups: Vec::new(),
        border: None,
        title_block: None,
        text_blocks: Vec::new(),
    };
    for stmt in &raw.statements {
        ctx.process_statement(stmt, None)?;
    }

    let mut groups: Vec<Group> = ctx
        .groups
        .iter()
        .map(|sg| Group {
            title: sg.title.clone(),
            direction: sg.direction,
            children: Vec::new(),
            parent: sg.parent,
            color: sg.color.clone(),
            fill: sg.fill.clone(),
            line: sg.line,
            text: sg.text.clone(),
        })
        .collect();
    // Populate each group's direct children (by parent link) in index order.
    for (idx, sg) in ctx.groups.iter().enumerate() {
        if let Some(p) = sg.parent {
            groups[p].children.push(idx);
        }
    }

    let nodes: Vec<Node> = ctx
        .nodes
        .iter()
        .map(|n| Node {
            id: n.id.clone(),
            label: n.label.clone().unwrap_or_else(|| n.id.clone()),
            shape: n.shape.unwrap_or(Shape::Box),
            color: n.color.clone(),
            fill: n.fill.clone(),
            text: n.text.clone(),
            group: n.membership,
        })
        .collect();

    check_corner_conflict(&ctx.title_block, &ctx.text_blocks)?;

    Ok(Diagram {
        direction: raw.direction,
        nodes,
        edges: ctx.edges,
        groups,
        border: ctx.border,
        title_block: ctx.title_block,
        text_blocks: ctx.text_blocks.into_iter().map(|(b, _)| b).collect(),
    })
}

/// The title block owns the lower-right corner of the page, so a text block
/// there has nowhere to go. Checked once, after every statement has been read,
/// because either statement may be written first — and the error always points
/// at the **text block**, since that is the statement that has to move (the
/// title block is the one reserved first).
fn check_corner_conflict(
    title_block: &Option<TitleBlock>,
    text_blocks: &[(TextBlock, usize)],
) -> Result<(), Error> {
    if title_block.is_none() {
        return Ok(());
    }
    if let Some((_, offset)) = text_blocks
        .iter()
        .find(|(b, _)| b.anchor == Anchor::LowerRight)
    {
        return Err(Error::Resolve {
            offset: *offset,
            message: "`textblock lower-right` conflicts with `titleblock` \
                      (the title block owns that corner)"
                .to_string(),
        });
    }
    Ok(())
}

impl Ctx {
    fn process_statement(
        &mut self,
        stmt: &RawStatement,
        group: Option<usize>,
    ) -> Result<(), Error> {
        match stmt {
            RawStatement::NodeList(nl) => self.process_nodelist(nl, group),
            RawStatement::Group(sg) => self.process_group(sg, group),
            // The parser rejects the page-furniture statements inside a group
            // body (they are page-level), so reaching one here with a `group`
            // would be a parser bug rather than user error.
            RawStatement::Border(rb) => {
                debug_assert!(group.is_none(), "a border is page-level");
                self.process_border(rb)
            }
            RawStatement::TitleBlock(rt) => {
                debug_assert!(group.is_none(), "a title block is page-level");
                self.process_title_block(rt)
            }
            RawStatement::TextBlock(rt) => {
                debug_assert!(group.is_none(), "a text block is page-level");
                self.process_text_block(rt)
            }
        }
    }

    // ---- page furniture (M21.1) ----
    //
    // Decoration only, so validation is all there is: no node, edge, or group
    // state is touched. Everything here errors with an offset (the resolve
    // layer's contract) so the CLI reports a line number.

    /// `border attr*` — at most one per diagram.
    fn process_border(&mut self, rb: &RawBorder) -> Result<(), Error> {
        validate_attrs(&rb.attrs, &["color", "line"], "border")?;
        let mut color = None;
        let mut line = None;
        for attr in &rb.attrs {
            match attr.name.as_str() {
                "color" => color = Some(attr.value.clone()),
                // `line` takes the same four style names a group's does, so a
                // typo like `line="wavy"` fails loudly instead of rendering
                // solid.
                "line" => {
                    line = Some(parse_style_value(
                        "border",
                        "line",
                        &attr.value,
                        attr.offset,
                    )?)
                }
                _ => unreachable!("validate_attrs ensures only known border attributes"),
            }
        }
        if self.border.is_some() {
            return Err(Error::Resolve {
                offset: rb.offset,
                message: "duplicate `border` (a diagram has at most one)".to_string(),
            });
        }
        self.border = Some(Border { color, line });
        Ok(())
    }

    /// `titleblock attr* field*` — at most one per diagram.
    ///
    /// `color` and `fill` are style keys; every other key is a **field** and is
    /// open by design (a title block is a metadata table whose keys the user
    /// chooses — the one documented exception to "unknown attribute is an
    /// error"). Keys are verbatim and ordered, so `Version` and `API Gateway`
    /// survive as written. A duplicate key is an error either way.
    fn process_title_block(&mut self, rt: &RawTitleBlock) -> Result<(), Error> {
        let mut seen: Vec<&str> = Vec::new();
        let mut fields = Vec::new();
        let mut color = None;
        let mut fill = None;
        for field in &rt.fields {
            if seen.contains(&field.name.as_str()) {
                return Err(Error::Resolve {
                    offset: field.offset,
                    message: format!("duplicate title block key `{}`", field.name),
                });
            }
            seen.push(&field.name);
            // The two style keys are not fields. They still occupy the key
            // namespace (a duplicate `color` is caught above), but they are
            // drawn as the table's stroke and interior, not as a row.
            match field.name.as_str() {
                "color" => color = Some(field.value.clone()),
                "fill" => fill = Some(field.value.clone()),
                _ => fields.push(Field {
                    key: field.name.clone(),
                    value: field.value.clone(),
                }),
            }
        }
        if self.title_block.is_some() {
            return Err(Error::Resolve {
                offset: rt.offset,
                message: "duplicate `titleblock` (a diagram has at most one)".to_string(),
            });
        }
        self.title_block = Some(TitleBlock {
            fields,
            color,
            fill,
        });
        Ok(())
    }

    /// `textblock anchor attr* rawline*` — one per anchor.
    fn process_text_block(&mut self, rt: &RawTextBlock) -> Result<(), Error> {
        validate_attrs(&rt.attrs, &["color", "fill", "text", "width"], "textblock")?;
        let mut color = None;
        let mut fill = None;
        let mut text = None;
        let mut width = None;
        for attr in &rt.attrs {
            match attr.name.as_str() {
                "color" => color = Some(attr.value.clone()),
                "fill" => fill = Some(attr.value.clone()),
                "text" => text = Some(attr.value.clone()),
                "width" => width = Some(parse_width(&attr.value, attr.offset)?),
                _ => unreachable!("validate_attrs ensures only known textblock attributes"),
            }
        }
        // A body of only blank lines is empty as far as the page is
        // concerned: it would draw nothing, so it is the same mistake as no
        // body at all (and the same message).
        if rt.body.iter().all(|line| line.trim().is_empty()) {
            return Err(Error::Resolve {
                offset: rt.offset,
                message: format!("text block at `{}` has an empty body", rt.anchor.as_str()),
            });
        }
        if let Some((existing, _)) = self.text_blocks.iter().find(|(b, _)| b.anchor == rt.anchor) {
            return Err(Error::Resolve {
                offset: rt.offset,
                message: format!(
                    "duplicate text block anchor `{}` (a text block at `{}` already exists)",
                    rt.anchor.as_str(),
                    existing.anchor.as_str()
                ),
            });
        }
        // The lower-right / title-block conflict is not checked here: either
        // statement may come first, so it is decided in one pass at the end
        // of `resolve` (see `check_corner_conflict`).
        self.text_blocks.push((
            TextBlock {
                anchor: rt.anchor,
                body: rt.body.clone(),
                color,
                fill,
                text,
                width,
            },
            rt.offset,
        ));
        Ok(())
    }

    fn process_group(&mut self, sg: &RawGroup, parent: Option<usize>) -> Result<(), Error> {
        // Per-group direction is now honored by layout (M4); the grammar
        // already validates the direction token, so we only carry it through.
        // Group style attributes (M7.5): `color` (border), `fill`
        // (background), `line` (border line style), and `text` (title color).
        validate_attrs(&sg.attrs, &["color", "fill", "line", "text"], "group")?;
        let mut color = None;
        let mut fill = None;
        let mut line = None;
        let mut text = None;
        for attr in &sg.attrs {
            match attr.name.as_str() {
                "color" => color = Some(attr.value.clone()),
                "fill" => fill = Some(attr.value.clone()),
                "line" => {
                    line = Some(parse_style_value(
                        "group",
                        "line",
                        &attr.value,
                        attr.offset,
                    )?)
                }
                "text" => text = Some(attr.value.clone()),
                _ => unreachable!("validate_attrs ensures only known group attributes"),
            }
        }
        let idx = self.groups.len();
        self.groups.push(GroupInfo {
            title: sg.title.clone(),
            direction: sg.direction,
            color,
            fill,
            line,
            text,
            parent,
        });
        for stmt in &sg.statements {
            self.process_statement(stmt, Some(idx))?;
        }
        Ok(())
    }

    fn process_nodelist(&mut self, nl: &RawNodeList, group: Option<usize>) -> Result<(), Error> {
        let is_standalone = nl.edges.is_empty();
        for n in &nl.nodes {
            self.register_node(n, is_standalone, group)?;
        }
        for (i, e) in nl.edges.iter().enumerate() {
            let from = nl.nodes[i].id.clone();
            let to = nl.nodes[i + 1].id.clone();
            // M11: `from` / `to` request the page-space side of the source /
            // target node the edge connects to (values `top | bottom | left
            // | right`, validated here so a typo like `from="rught"` fails
            // loudly instead of silently falling back to the implicit
            // choice).
            validate_attrs(&e.attrs, &["color", "text", "from", "to"], "edge")?;
            let mut color = None;
            let mut text = None;
            let mut from_side = None;
            let mut to_side = None;
            for attr in &e.attrs {
                match attr.name.as_str() {
                    "color" => color = Some(attr.value.clone()),
                    "text" => text = Some(attr.value.clone()),
                    "from" => from_side = Some(parse_edge_side("from", &attr.value, attr.offset)?),
                    "to" => to_side = Some(parse_edge_side("to", &attr.value, attr.offset)?),
                    _ => unreachable!("validate_attrs ensures only known edge attributes"),
                }
            }
            self.edges.push(Edge {
                from,
                to,
                style: e.style.unwrap_or(Style::Solid),
                label: e.label.clone(),
                color,
                text,
                from_side,
                to_side,
            });
        }
        Ok(())
    }

    fn register_node(
        &mut self,
        occ: &RawNodeDecl,
        is_standalone: bool,
        group: Option<usize>,
    ) -> Result<(), Error> {
        validate_attrs(&occ.attrs, &["color", "fill", "text"], "node")?;
        if let Some(&idx) = self.id_index.get(&occ.id) {
            // Existing node: merge and check consistency.
            let info = &mut self.nodes[idx];
            if let Some(lbl) = &occ.label {
                match &info.label {
                    None => info.label = Some(lbl.clone()),
                    Some(ex) if ex == lbl => {}
                    Some(_) => {
                        return Err(Error::Resolve {
                            offset: occ.offset,
                            message: format!(
                                "node `{}` redeclared with a conflicting label",
                                occ.id
                            ),
                        });
                    }
                }
            }
            if let Some(sh) = occ.shape {
                match info.shape {
                    None => info.shape = Some(sh),
                    Some(ex) if ex == sh => {}
                    Some(_) => {
                        return Err(Error::Resolve {
                            offset: occ.offset,
                            message: format!(
                                "node `{}` redeclared with a conflicting shape",
                                occ.id
                            ),
                        });
                    }
                }
            }
            for attr in &occ.attrs {
                match attr.name.as_str() {
                    "color" => {
                        merge_attr(&mut info.color, &attr.value, attr.offset, &occ.id, "color")?
                    }
                    "fill" => {
                        merge_attr(&mut info.fill, &attr.value, attr.offset, &occ.id, "fill")?
                    }
                    "text" => {
                        merge_attr(&mut info.text, &attr.value, attr.offset, &occ.id, "text")?
                    }
                    _ => unreachable!("validate_attrs ensures only known node attributes"),
                }
            }
            // Group placement: an occurrence inside a `group` body places
            // the node in that group — standalone declarations and
            // edge-chain endpoints alike (a chain in a group body declares
            // its nodes as members). The one exception keeps cross-boundary
            // edges writable from inside a body: an edge endpoint naming a
            // node already placed in a *different* group is a cross-boundary
            // reference and leaves the node where it is (an outer body's
            // chain may target a nested group's member). A standalone
            // declaration in a second group is still an error — it could
            // only mean re-placement. Occurrences at top level never move a
            // node: membership is decided by group bodies alone, so source
            // order cannot silently un-group a node.
            if let Some(g) = group
                && !info.placed_groups.contains(&g)
            {
                let cross_boundary_ref = !is_standalone && !info.placed_groups.is_empty();
                if !cross_boundary_ref {
                    info.placed_groups.push(g);
                    if info.placed_groups.len() >= 2 {
                        return Err(Error::Resolve {
                            offset: occ.offset,
                            message: format!("node `{}` appears in more than one group", occ.id),
                        });
                    }
                    info.membership = Some(g);
                }
            }
        } else {
            // New node: implicitly declared in the current group.
            let mut color = None;
            let mut fill = None;
            let mut text = None;
            for attr in &occ.attrs {
                match attr.name.as_str() {
                    "color" => color = Some(attr.value.clone()),
                    "fill" => fill = Some(attr.value.clone()),
                    "text" => text = Some(attr.value.clone()),
                    _ => unreachable!("validate_attrs ensures only known node attributes"),
                }
            }
            let mut placed_groups = Vec::new();
            if let Some(g) = group {
                placed_groups.push(g);
            }
            let idx = self.nodes.len();
            self.nodes.push(NodeInfo {
                id: occ.id.clone(),
                label: occ.label.clone(),
                shape: occ.shape,
                color,
                fill,
                text,
                membership: group,
                placed_groups,
            });
            self.id_index.insert(occ.id.clone(), idx);
        }
        Ok(())
    }
}

fn merge_attr(
    slot: &mut Option<String>,
    value: &str,
    offset: usize,
    id: &str,
    name: &str,
) -> Result<(), Error> {
    match slot {
        None => {
            *slot = Some(value.to_string());
            Ok(())
        }
        Some(ex) if ex == value => Ok(()),
        Some(_) => Err(Error::Resolve {
            offset,
            message: format!(
                "node `{}` redeclared with a conflicting value for `{name}`",
                id
            ),
        }),
    }
}

/// Parse a `line` attribute's value into a [`Style`]. The value is a quoted
/// string (so `"dashed"`, not the bare contextual keyword an edge body uses),
/// and must be one of the four style names; anything else is a resolve error
/// so a typo like `line="wavy"` fails loudly instead of silently rendering as
/// a solid frame. `owner` names the position in the message ("group",
/// "border").
fn parse_style_value(owner: &str, name: &str, value: &str, offset: usize) -> Result<Style, Error> {
    Style::from_ident(value).ok_or_else(|| Error::Resolve {
        offset,
        message: format!(
            "{owner} `{name}` must be one of solid, dotted, dashed, thick; got `{value}`"
        ),
    })
}

/// Parse a text block's `width` attribute: a **positive** number of pixels.
///
/// The value arrives as a quoted string (`width="320"`), so it is parsed here
/// rather than by the lexer. Zero and negatives are rejected — a width of zero
/// would wrap every word onto its own line and a negative one has no meaning —
/// and so is anything that is not a finite number, so `width="wide"` fails
/// with a message that names the expected shape instead of silently rendering
/// an unwrapped block.
fn parse_width(value: &str, offset: usize) -> Result<f32, Error> {
    let bad = || Error::Resolve {
        offset,
        message: format!("textblock `width` must be a positive number; got `{value}`"),
    };
    let w: f32 = value.trim().parse().map_err(|_| bad())?;
    if !w.is_finite() || w <= 0.0 {
        return Err(bad());
    }
    Ok(w)
}

/// Parse the value of an edge `from` / `to` attribute into an
/// [`EdgeSide`]. The value is a quoted string (so `from="left"`, not the
/// bare word), and must be one of the four page-space side names;
/// anything else is a resolve error (the same strict-attribute convention
/// as the group `line` style).
fn parse_edge_side(name: &str, value: &str, offset: usize) -> Result<EdgeSide, Error> {
    EdgeSide::from_ident(value).ok_or_else(|| Error::Resolve {
        offset,
        message: format!("edge `{name}` must be one of top, bottom, left, right; got `{value}`"),
    })
}

/// Validate that every attribute name is allowed for this position and that
/// no attribute name repeats within a single declaration.
fn validate_attrs(attrs: &[RawAttr], allowed: &[&str], pos_name: &str) -> Result<(), Error> {
    let mut seen: Vec<String> = Vec::new();
    for attr in attrs {
        if !allowed.contains(&attr.name.as_str()) {
            return Err(Error::Resolve {
                offset: attr.offset,
                message: format!("unknown {} attribute `{}`", pos_name, attr.name),
            });
        }
        if seen.contains(&attr.name) {
            return Err(Error::Resolve {
                offset: attr.offset,
                message: format!("duplicate attribute `{}`", attr.name),
            });
        }
        seen.push(attr.name.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::line_of;
    use crate::parser;

    fn ok(src: &str) -> Diagram {
        let raw = parser::parse_diagram(src).expect("parse should succeed");
        resolve(&raw).expect("resolve should succeed")
    }

    fn err(src: &str) -> (Error, usize) {
        let source = src;
        match parser::parse_diagram(source) {
            Ok(raw) => match resolve(&raw) {
                Ok(_) => panic!("expected an error, got success"),
                Err(e) => {
                    let line = error_line(source, &e);
                    (e, line)
                }
            },
            Err(e) => {
                let line = error_line(source, &e);
                (e, line)
            }
        }
    }

    fn error_line(source: &str, e: &Error) -> usize {
        match e {
            Error::Parse { offset, .. } | Error::Resolve { offset, .. } => line_of(source, *offset),
            Error::Io(_) => 0,
        }
    }

    fn node<'a>(d: &'a Diagram, id: &str) -> &'a Node {
        d.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| {
            panic!(
                "no node `{id}`; have {:?}",
                d.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
            )
        })
    }

    /// The ids of a group's direct members, derived from the nodes'
    /// membership (the model's single source of truth: `Node::group`). A
    /// group's members are the nodes whose `group` is this index, in
    /// declaration order.
    fn members(d: &Diagram, group: usize) -> Vec<&str> {
        d.nodes
            .iter()
            .filter(|n| n.group == Some(group))
            .map(|n| n.id.as_str())
            .collect()
    }

    #[test]
    fn basic_counts() {
        let d = ok(include_str!("../examples/infra.dgmr"));
        assert_eq!(d.direction, Direction::TopDown);
        assert_eq!(d.nodes.len(), 8);
        assert_eq!(d.edges.len(), 10);
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].title.as_deref(), Some("Kubernetes Cluster"));
        assert_eq!(members(&d, 0), ["api1", "api2"]);
    }

    #[test]
    fn implicit_declaration_in_edge_chain() {
        let d = ok("diagram left-right\na \"Web\" --> lb \"LB\" --> api1\n");
        assert_eq!(d.nodes.len(), 3);
        assert_eq!(node(&d, "lb").label, "LB");
        assert_eq!(d.edges.len(), 2);
        assert_eq!(d.edges[0].from, "a");
        assert_eq!(d.edges[0].to, "lb");
    }

    #[test]
    fn bare_id_uses_id_as_label() {
        let d = ok("diagram top-down\nweb\n");
        assert_eq!(node(&d, "web").label, "web");
        assert_eq!(node(&d, "web").shape, Shape::Box);
    }

    #[test]
    fn shapes_and_colors() {
        let d = ok("diagram top-down\ndb \"Pg\" : cylinder color=\"#888\" fill=\"#eef\"\n");
        let db = node(&d, "db");
        assert_eq!(db.shape, Shape::Cylinder);
        assert_eq!(db.color.as_deref(), Some("#888"));
        assert_eq!(db.fill.as_deref(), Some("#eef"));
    }

    #[test]
    fn edge_styles_labels_and_colors() {
        let d = ok("diagram top-down\n\
             a -- dotted \"sync\" color=\"#888\" --> b\n\
             c -- thick --> d\n\
             e -- \"only a label\" --> f\n");
        assert_eq!(d.edges[0].style, Style::Dotted);
        assert_eq!(d.edges[0].label.as_deref(), Some("sync"));
        assert_eq!(d.edges[0].color.as_deref(), Some("#888"));
        assert_eq!(d.edges[1].style, Style::Thick);
        assert_eq!(d.edges[2].style, Style::Solid); // label but no style
    }

    #[test]
    fn hyphenated_ids_do_not_eat_edge_operator() {
        // `A-->B` and `api-server` must both work.
        let d = ok("diagram top-down\nA-->B\napi-server \"API\"\n");
        assert_eq!(d.nodes.len(), 3);
        assert_eq!(d.edges[0].from, "A");
        assert_eq!(d.edges[0].to, "B");
        assert_eq!(node(&d, "api-server").label, "API");
    }

    #[test]
    fn move_top_level_node_into_group() {
        let d = ok("diagram top-down\n\
             a \"A\"\n\
             group \"S\"\n\
             a\n\
             end\n");
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(members(&d, 0), ["a"]);
    }

    #[test]
    fn cross_boundary_edge_does_not_relocate_node() {
        let d = ok("diagram top-down\n\
             group \"S\"\n\
             a\n\
             end\n\
             b --> a\n");
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(node(&d, "b").group, None);
        assert_eq!(d.edges.len(), 1);
        assert_eq!(d.edges[0].from, "b");
        assert_eq!(d.edges[0].to, "a");
    }

    #[test]
    fn edge_chain_in_group_brings_predeclared_nodes_into_the_group() {
        // The `sides.dgmr` shape: labels declared at top level, structure
        // given by an edge chain inside the group body. The chain's
        // endpoints become members of the group they're declared in.
        let d = ok("diagram top-down\n\
             bus \"Event Bus\"\n\
             queue \"Queue\"\n\
             group left-right \"Messaging\"\n\
             bus -- from=\"right\" to=\"left\" --> queue\n\
             end\n");
        assert_eq!(node(&d, "bus").group, Some(0));
        assert_eq!(node(&d, "queue").group, Some(0));
        assert_eq!(members(&d, 0), ["bus", "queue"]);
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        assert_eq!(d.edges.len(), 1);
    }

    #[test]
    fn edge_chain_nodes_first_seen_in_a_group_are_members() {
        let d = ok("diagram top-down\n\
             group left-right \"S\"\n\
             a --> b\n\
             end\n");
        assert_eq!(members(&d, 0), ["a", "b"]);
    }

    #[test]
    fn top_level_declarations_after_a_group_do_not_un_group_its_members() {
        // Membership is decided by group bodies alone: a later top-level
        // declaration only merges label/shape/attributes, so "structure
        // first, labels later" works as well as the reverse ordering.
        let d = ok("diagram top-down\n\
             group \"Messaging\"\n\
             bus --> queue\n\
             end\n\
             bus \"Event Bus\"\n\
             queue \"Queue\"\n");
        assert_eq!(node(&d, "bus").group, Some(0));
        assert_eq!(node(&d, "queue").group, Some(0));
        assert_eq!(members(&d, 0), ["bus", "queue"]);
        assert_eq!(node(&d, "bus").label, "Event Bus");
        assert_eq!(node(&d, "queue").label, "Queue");
    }

    #[test]
    fn edge_chain_in_group_body_can_reference_another_groups_node() {
        // A cross-boundary reference: `b --> a` inside B's body names `a`,
        // which already belongs to A. The edge is cross-boundary; `a` keeps
        // its group (an edge endpoint never relocates an already-placed
        // node) and only `b` becomes a member of B.
        let d = ok("diagram top-down\n\
             group \"A\"\n\
             a\n\
             end\n\
             group \"B\"\n\
             b --> a\n\
             end\n");
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(node(&d, "b").group, Some(1));
        assert_eq!(members(&d, 0), ["a"]);
        assert_eq!(members(&d, 1), ["b"]);
        assert_eq!(d.edges.len(), 1);
    }

    #[test]
    fn outer_body_chain_targeting_inner_member_keeps_nesting() {
        // The within-frame cross-boundary shape: `x --> a` written in Outer's
        // body targets `a` inside the nested Inner group; `a` must stay in
        // Inner and only `x` joins Outer.
        let d = ok("diagram top-down\n\
             group \"Outer\"\n\
             x\n\
             group \"Inner\"\n\
             a\n\
             end\n\
             x --> a\n\
             end\n");
        assert_eq!(node(&d, "x").group, Some(0));
        assert_eq!(node(&d, "a").group, Some(1));
        assert_eq!(members(&d, 0), ["x"]);
        assert_eq!(members(&d, 1), ["a"]);
    }

    #[test]
    fn nested_groups_record_containment() {
        let d = ok("diagram top-down\n\
             group \"Outer\"\n\
             group \"Inner\"\n\
             a\n\
             end\n\
             end\n");
        assert_eq!(d.groups.len(), 2);
        assert_eq!(node(&d, "a").group, Some(1)); // innermost
        // Containment tree: Outer contains Inner; both are top-level siblings'
        // parent links.
        assert_eq!(d.groups[0].parent, None);
        assert_eq!(d.groups[0].children, [1]);
        assert_eq!(d.groups[1].parent, Some(0));
        assert!(d.groups[1].children.is_empty());
        assert!(members(&d, 0).is_empty());
        assert_eq!(members(&d, 1), ["a"]);
    }

    // ---- error cases ----

    #[test]
    fn node_in_two_groups_is_an_error() {
        let (e, line) = err("diagram top-down\n\
             group \"A\"\n\
             a\n\
             end\n\
             group \"B\"\n\
             a\n\
             end\n");
        assert_eq!(line, 6);
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("appears in more than one group"))
        );
    }

    #[test]
    fn conflicting_label_is_an_error() {
        let (e, line) = err("diagram top-down\na \"X\"\na \"Y\"\n");
        assert_eq!(line, 3);
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("conflicting label"))
        );
    }

    #[test]
    fn same_redeclaration_is_ok() {
        let d = ok("diagram top-down\na \"X\"\na \"X\"\n");
        assert_eq!(node(&d, "a").label, "X");
    }

    #[test]
    fn unknown_node_attribute_is_an_error() {
        let (e, line) = err("diagram top-down\na bogus=\"x\"\n");
        assert_eq!(line, 2);
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown node attribute `bogus`"))
        );
    }

    #[test]
    fn unknown_edge_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na -- bogus=\"x\" --> b\n");
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown edge attribute `bogus`"))
        );
    }

    #[test]
    fn duplicate_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na color=\"x\" color=\"y\"\n");
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `color`"))
        );
    }

    #[test]
    fn group_direction_is_accepted() {
        // M4: per-group direction is now honored, not rejected.
        let d = ok("diagram top-down\n\
             group left-right \"S\"\n\
             a\n\
             end\n");
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        // A group with no direction inherits at layout time (stored as None).
        let d2 = ok("diagram top-down\n\
             group \"T\"\n\
             b\n\
             end\n");
        assert_eq!(d2.groups[0].direction, None);
    }

    #[test]
    fn missing_diagram_header_is_a_parse_error() {
        let (e, line) = err("flowchart top-down\n");
        assert_eq!(line, 1);
        assert!(
            matches!(e, Error::Parse { ref message, .. } if message.contains("diagram header"))
        );
    }

    #[test]
    fn bad_direction_is_a_parse_error() {
        let (e, line) = err("diagram sideways\n");
        assert_eq!(line, 1);
        assert!(
            matches!(e, Error::Parse { ref message, .. } if message.contains("expected direction"))
        );
    }

    #[test]
    fn unterminated_string_is_a_parse_error() {
        let (e, _line) = err("diagram top-down\na \"oops\n");
        assert!(
            matches!(e, Error::Parse { ref message, .. } if message.contains("unterminated string literal"))
        );
    }

    #[test]
    fn unexpected_end_is_a_parse_error() {
        let (e, line) = err("diagram top-down\nend\n");
        assert_eq!(line, 2);
        assert!(
            matches!(e, Error::Parse { ref message, .. } if message.contains("unexpected `end`"))
        );
    }

    // ---- M7.5: group color/fill/line/text and text-color attributes ----

    #[test]
    fn group_style_attributes_are_parsed() {
        let d = ok("diagram top-down\n\
             group \"S\" color=\"#888\" fill=\"#eef\" line=\"dashed\" text=\"#005\"\n\
             a\n\
             end\n");
        assert_eq!(d.groups.len(), 1);
        let sg = &d.groups[0];
        assert_eq!(sg.color.as_deref(), Some("#888"));
        assert_eq!(sg.fill.as_deref(), Some("#eef"));
        assert_eq!(sg.line, Some(Style::Dashed));
        assert_eq!(sg.text.as_deref(), Some("#005"));
    }

    #[test]
    fn group_line_accepts_all_styles() {
        for (s, want) in [
            ("solid", Style::Solid),
            ("dotted", Style::Dotted),
            ("dashed", Style::Dashed),
            ("thick", Style::Thick),
        ] {
            let src = format!("diagram top-down\ngroup \"S\" line=\"{s}\"\na\nend\n");
            let d = ok(&src);
            assert_eq!(d.groups[0].line, Some(want), "line=\"{s}\"");
        }
    }

    #[test]
    fn group_invalid_line_style_is_an_error() {
        let (e, line) = err("diagram top-down\n\
             group \"S\" line=\"wavy\"\n\
             a\n\
             end\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("group `line` must be one of") && message.contains("wavy")));
    }

    #[test]
    fn group_unknown_attribute_is_an_error() {
        let (e, line) = err("diagram top-down\n\
             group \"S\" bogus=\"x\"\n\
             a\n\
             end\n");
        assert_eq!(line, 2);
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown group attribute `bogus`"))
        );
    }

    #[test]
    fn group_duplicate_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\n\
             group \"S\" color=\"x\" color=\"y\"\n\
             a\n\
             end\n");
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `color`"))
        );
    }

    #[test]
    fn node_text_attribute_is_parsed() {
        let d = ok("diagram top-down\na \"A\" text=\"#005\"\n");
        assert_eq!(node(&d, "a").text.as_deref(), Some("#005"));
    }

    #[test]
    fn edge_text_attribute_is_parsed() {
        let d = ok("diagram top-down\na -- \"sync\" text=\"#005\" --> b\n");
        assert_eq!(d.edges[0].text.as_deref(), Some("#005"));
        assert_eq!(d.edges[0].label.as_deref(), Some("sync"));
    }

    #[test]
    fn node_text_conflict_on_redeclaration_is_an_error() {
        let (e, line) = err("diagram top-down\na text=\"#005\"\na text=\"#006\"\n");
        assert_eq!(line, 3);
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("conflicting value for `text`"))
        );
    }

    #[test]
    fn node_text_redeclared_consistently_is_ok() {
        let d = ok("diagram top-down\na text=\"#005\"\na text=\"#005\"\n");
        assert_eq!(node(&d, "a").text.as_deref(), Some("#005"));
    }

    #[test]
    fn group_style_attrs_do_not_perturb_structure() {
        // Style attributes are render-only; they must not change the resolved
        // structural fields (direction, members, containment).
        let d = ok("diagram top-down\n\
             group left-right \"S\" color=\"#888\" fill=\"#eef\" line=\"dashed\" text=\"#005\"\n\
             a\n\
             b\n\
             end\n");
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        assert_eq!(members(&d, 0), ["a", "b"]);
    }

    // ---- M11: edge side attributes (`from=` / `to=`) ----

    #[test]
    fn edge_side_attributes_are_parsed() {
        let d = ok("diagram top-down\na -- from=\"right\" to=\"top\" --> b\n");
        assert_eq!(d.edges[0].from_side, Some(EdgeSide::Right));
        assert_eq!(d.edges[0].to_side, Some(EdgeSide::Top));
        // Unset sides stay None (the layout engine's implicit choice).
        let d2 = ok("diagram top-down\na --> b\n");
        assert_eq!(d2.edges[0].from_side, None);
        assert_eq!(d2.edges[0].to_side, None);
    }

    #[test]
    fn edge_side_accepts_all_sides() {
        for (s, want) in [
            ("top", EdgeSide::Top),
            ("bottom", EdgeSide::Bottom),
            ("left", EdgeSide::Left),
            ("right", EdgeSide::Right),
        ] {
            let src = format!("diagram top-down\na -- from=\"{s}\" --> b\n");
            let d = ok(&src);
            assert_eq!(d.edges[0].from_side, Some(want), "from=\"{s}\"");
            let src = format!("diagram top-down\na -- to=\"{s}\" --> b\n");
            let d = ok(&src);
            assert_eq!(d.edges[0].to_side, Some(want), "to=\"{s}\"");
        }
    }

    #[test]
    fn edge_side_invalid_value_is_an_error() {
        let (e, line) = err("diagram top-down\na -- from=\"rught\" --> b\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("edge `from` must be one of") && message.contains("rught")));
        let (e, line) = err("diagram top-down\na -- to=\"sideways\" --> b\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("edge `to` must be one of") && message.contains("sideways")));
    }

    #[test]
    fn edge_side_duplicate_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na -- from=\"left\" from=\"right\" --> b\n");
        assert!(
            matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `from`"))
        );
    }

    // ---- M21.1: page furniture (border / titleblock / textblock) ----

    #[test]
    fn page_furniture_absent_by_default() {
        // The property that keeps M21.1 output-neutral: a diagram that uses
        // none of it resolves to the same empty decoration as before.
        let d = ok(include_str!("../examples/infra.dgmr"));
        assert!(d.border.is_none());
        assert!(d.title_block.is_none());
        assert!(d.text_blocks.is_empty());
    }

    #[test]
    fn border_resolves_with_its_style() {
        let d = ok("diagram top-down\nborder color=\"#333\" line=\"dashed\"\n");
        let b = d.border.as_ref().expect("border");
        assert_eq!(b.color.as_deref(), Some("#333"));
        assert_eq!(b.line, Some(Style::Dashed));
        // Bare `border` is present with default styling.
        let d = ok("diagram top-down\nborder\n");
        let b = d.border.as_ref().expect("border");
        assert_eq!(b.color, None);
        assert_eq!(b.line, None);
    }

    #[test]
    fn border_accepts_all_line_styles() {
        for (s, want) in [
            ("solid", Style::Solid),
            ("dotted", Style::Dotted),
            ("dashed", Style::Dashed),
            ("thick", Style::Thick),
        ] {
            let d = ok(&format!("diagram top-down\nborder line=\"{s}\"\n"));
            assert_eq!(d.border.expect("border").line, Some(want), "line=\"{s}\"");
        }
    }

    #[test]
    fn border_color_empty_string_is_a_value_not_an_error() {
        // `color=""` draws nothing — a legal way to reserve the inset without
        // stroking it.
        let d = ok("diagram top-down\nborder color=\"\"\n");
        assert_eq!(d.border.expect("border").color.as_deref(), Some(""));
    }

    #[test]
    fn duplicate_border_is_an_error() {
        let (e, line) = err("diagram top-down\nborder\nborder\n");
        assert_eq!(line, 3);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate `border`")));
    }

    #[test]
    fn border_invalid_line_style_is_an_error() {
        let (e, line) = err("diagram top-down\nborder line=\"wavy\"\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("border `line` must be one of") && message.contains("wavy")));
    }

    #[test]
    fn border_unknown_attribute_is_an_error() {
        let (e, line) = err("diagram top-down\nborder bogus=\"x\"\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("unknown border attribute `bogus`")));
    }

    #[test]
    fn border_duplicate_attribute_is_an_error() {
        let (e, _) = err("diagram top-down\nborder color=\"a\" color=\"b\"\n");
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate attribute `color`")));
    }

    #[test]
    fn title_block_keeps_its_fields_in_order_and_verbatim() {
        let d = ok("diagram top-down\ntitleblock\n\
             Title   = \"Finance: Network Architecture\"\n\
             Company = \"Big Corp, LLC\"\n\
             API     = \"v2\"\n\
             end\n");
        let tb = d.title_block.as_ref().expect("title block");
        let pairs: Vec<(&str, &str)> = tb
            .fields
            .iter()
            .map(|f| (f.key.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("Title", "Finance: Network Architecture"),
                ("Company", "Big Corp, LLC"),
                // Keys are open by design: an unknown key is data, not a typo,
                // and capitalization is the author's.
                ("API", "v2"),
            ]
        );
        assert_eq!(tb.color, None);
        assert_eq!(tb.fill, None);
    }

    #[test]
    fn title_block_style_keys_are_not_fields() {
        let d = ok("diagram top-down\ntitleblock\n\
             Title = \"X\"\n\
             color = \"#888\"\n\
             fill  = \"#eef\"\n\
             end\n");
        let tb = d.title_block.as_ref().expect("title block");
        assert_eq!(tb.fields.len(), 1, "color/fill are style, not fields");
        assert_eq!(tb.fields[0].key, "Title");
        assert_eq!(tb.color.as_deref(), Some("#888"));
        assert_eq!(tb.fill.as_deref(), Some("#eef"));
    }

    #[test]
    fn title_block_style_keys_occupy_the_key_namespace() {
        // `color` is not a field, but a repeated `color` is still a duplicate
        // key (it would otherwise be a silently last-one-wins style).
        let (e, line) =
            err("diagram top-down\ntitleblock\ncolor = \"#111\"\ncolor = \"#222\"\nend\n");
        assert_eq!(line, 4);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate title block key `color`")));
    }

    #[test]
    fn title_block_one_liner_resolves() {
        let d = ok("diagram top-down\ntitleblock Title=\"X\" Company=\"Y\"\nend\n");
        let tb = d.title_block.as_ref().expect("title block");
        assert_eq!(tb.fields.len(), 2);
        assert_eq!(tb.fields[1].key, "Company");
    }

    #[test]
    fn empty_title_block_is_legal() {
        // Unlike a text block, an empty title block is well-defined: the
        // inset is reserved and the table is empty (M21.3 decides how it
        // draws). The grammar allows it, so resolve does too.
        let d = ok("diagram top-down\ntitleblock\nend\n");
        assert!(d.title_block.expect("title block").fields.is_empty());
    }

    #[test]
    fn duplicate_title_block_key_is_an_error() {
        let (e, line) = err("diagram top-down\ntitleblock\nTitle = \"X\"\nTitle = \"Y\"\nend\n");
        assert_eq!(line, 4);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate title block key `Title`")));
    }

    #[test]
    fn duplicate_title_block_is_an_error() {
        let (e, line) = err("diagram top-down\ntitleblock\nTitle = \"X\"\nend\n\
             titleblock\nCompany = \"Y\"\nend\n");
        assert_eq!(line, 5);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate `titleblock`")));
    }

    #[test]
    fn text_block_resolves_body_anchor_and_style() {
        let d = ok("diagram top-down\n\
             textblock upper-left width=\"320\" color=\"#333\" fill=\"#eef\" text=\"#005\"\n\
             line one\n\
             \n\
             line two\n\
             end\n");
        assert_eq!(d.text_blocks.len(), 1);
        let tb = &d.text_blocks[0];
        assert_eq!(tb.anchor, Anchor::UpperLeft);
        assert_eq!(tb.body, ["line one", "", "line two"]);
        assert_eq!(tb.width, Some(320.0));
        assert_eq!(tb.color.as_deref(), Some("#333"));
        assert_eq!(tb.fill.as_deref(), Some("#eef"));
        assert_eq!(tb.text.as_deref(), Some("#005"));
    }

    #[test]
    fn every_anchor_may_hold_one_block() {
        let d = ok("diagram top-down\n\
             textblock top\na\nend\n\
             textblock upper-right\nb\nend\n\
             textblock right\nc\nend\n\
             textblock lower-right\nd\nend\n\
             textblock bottom\ne\nend\n\
             textblock lower-left\nf\nend\n\
             textblock left\ng\nend\n\
             textblock upper-left\nh\nend\n");
        assert_eq!(d.text_blocks.len(), 8);
        assert_eq!(d.text_blocks[3].anchor, Anchor::LowerRight);
    }

    #[test]
    fn duplicate_text_block_anchor_is_an_error() {
        let (e, line) = err("diagram top-down\n\
             textblock right\na\nend\n\
             textblock right\nb\nend\n");
        assert_eq!(line, 5);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate text block anchor `right`")));
    }

    #[test]
    fn textblock_lower_right_conflicts_with_titleblock() {
        // The title block owns the lower-right corner, and it is reserved
        // first — so the conflict is reported on the text block whichever
        // order they are written in.
        let (e, line) = err("diagram top-down\n\
             titleblock\nTitle = \"X\"\nend\n\
             textblock lower-right\na\nend\n");
        assert_eq!(line, 5);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("`textblock lower-right` conflicts with `titleblock`")));
        let (e, line) = err("diagram top-down\n\
             textblock lower-right\na\nend\n\
             titleblock\nTitle = \"X\"\nend\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("conflicts with `titleblock`")));
    }

    #[test]
    fn empty_text_block_body_is_an_error() {
        let (e, line) = err("diagram top-down\ntextblock right\nend\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("text block at `right` has an empty body")));
    }

    #[test]
    fn whitespace_only_text_block_body_is_empty() {
        // A body of only blank lines would draw nothing, so it is the same
        // mistake as no body at all.
        let (e, line) = err("diagram top-down\ntextblock right\n   \n\t\nend\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("has an empty body")));
    }

    #[test]
    fn text_block_width_must_be_a_positive_number() {
        for bad in ["0", "-10", "wide", "", "NaN", "inf", "1e999"] {
            let src = format!("diagram top-down\ntextblock right width=\"{bad}\"\na\nend\n");
            let (e, line) = err(&src);
            assert_eq!(line, 2, "width=\"{bad}\"");
            assert!(
                matches!(e, Error::Resolve { ref message, .. }
                    if message.contains("textblock `width` must be a positive number")),
                "width=\"{bad}\" gave {e:?}"
            );
        }
    }

    #[test]
    fn text_block_width_accepts_a_fractional_number() {
        let d = ok("diagram top-down\ntextblock right width=\"320.5\"\na\nend\n");
        assert_eq!(d.text_blocks[0].width, Some(320.5));
    }

    #[test]
    fn text_block_unknown_attribute_is_an_error() {
        // The title block's keys are open; a text block's attributes are not
        // (`color`/`fill`/`text`/`width` only), so a typo still fails loudly.
        let (e, line) = err("diagram top-down\ntextblock right bogus=\"x\"\na\nend\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("unknown textblock attribute `bogus`")));
    }

    #[test]
    fn text_block_duplicate_attribute_is_an_error() {
        let (e, _) = err("diagram top-down\ntextblock right width=\"10\" width=\"20\"\na\nend\n");
        assert!(matches!(e, Error::Resolve { ref message, .. }
                if message.contains("duplicate attribute `width`")));
    }

    #[test]
    fn page_furniture_does_not_perturb_the_graph() {
        // Decoration only: adding all three changes no node, edge, group, or
        // membership — which is what makes M21.1 output-neutral.
        let plain = ok("diagram top-down\nweb --> lb\ngroup \"G\"\napi\nend\n");
        let decorated = ok("diagram top-down\n\
             border color=\"#333\"\n\
             titleblock\nTitle = \"X\"\nend\n\
             textblock right\nhello\nend\n\
             web --> lb\ngroup \"G\"\napi\nend\n");
        assert_eq!(decorated.nodes.len(), plain.nodes.len());
        assert_eq!(decorated.edges.len(), plain.edges.len());
        assert_eq!(decorated.groups.len(), plain.groups.len());
        for (a, b) in decorated.nodes.iter().zip(&plain.nodes) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.group, b.group);
            assert_eq!(a.label, b.label);
        }
        assert_eq!(decorated.edges[0].from, plain.edges[0].from);
        assert_eq!(decorated.edges[0].to, plain.edges[0].to);
        assert_eq!(decorated.direction, plain.direction);
    }

    #[test]
    fn edge_side_attributes_combine_with_other_edge_attrs() {
        let d = ok("diagram top-down\n\
             a -- thick \"sync\" color=\"#888\" from=\"left\" to=\"bottom\" --> b\n");
        let e = &d.edges[0];
        assert_eq!(e.style, Style::Thick);
        assert_eq!(e.label.as_deref(), Some("sync"));
        assert_eq!(e.color.as_deref(), Some("#888"));
        assert_eq!(e.from_side, Some(EdgeSide::Left));
        assert_eq!(e.to_side, Some(EdgeSide::Bottom));
    }
}
