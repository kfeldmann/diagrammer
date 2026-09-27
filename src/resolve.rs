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
    #[allow(dead_code)]
    offset: usize,
}

struct Ctx {
    nodes: Vec<NodeInfo>,
    id_index: HashMap<String, usize>,
    edges: Vec<Edge>,
    groups: Vec<GroupInfo>,
}

pub fn resolve(raw: &RawDiagram) -> Result<Diagram, Error> {
    let mut ctx = Ctx {
        nodes: Vec::new(),
        id_index: HashMap::new(),
        edges: Vec::new(),
        groups: Vec::new(),
    };
    for stmt in &raw.statements {
        ctx.process_statement(stmt, None)?;
    }

    let groups: Vec<Group> = ctx
        .groups
        .iter()
        .enumerate()
        .map(|(idx, sg)| {
            let members: Vec<String> = ctx
                .nodes
                .iter()
                .filter(|n| n.membership == Some(idx))
                .map(|n| n.id.clone())
                .collect();
            Group {
                title: sg.title.clone(),
                direction: sg.direction,
                members,
                children: Vec::new(),
                parent: sg.parent,
                color: sg.color.clone(),
                fill: sg.fill.clone(),
                line: sg.line,
                text: sg.text.clone(),
            }
        })
        .collect();
    // Populate each group's direct children (by parent link) in index order.
    let mut groups = groups;
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

    Ok(Diagram {
        direction: raw.direction,
        nodes,
        edges: ctx.edges,
        groups,
    })
}

impl Ctx {
    fn process_statement(&mut self, stmt: &RawStatement, group: Option<usize>) -> Result<(), Error> {
        match stmt {
            RawStatement::NodeList(nl) => self.process_nodelist(nl, group),
            RawStatement::Group(sg) => self.process_group(sg, group),
        }
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
                "line" => line = Some(parse_line_style(&attr.value, attr.offset)?),
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
            offset: sg.offset,
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
                    "color" => merge_attr(&mut info.color, &attr.value, attr.offset, &occ.id, "color")?,
                    "fill" => merge_attr(&mut info.fill, &attr.value, attr.offset, &occ.id, "fill")?,
                    "text" => merge_attr(&mut info.text, &attr.value, attr.offset, &occ.id, "text")?,
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
                            message: format!(
                                "node `{}` appears in more than one group",
                                occ.id
                            ),
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
            message: format!("node `{}` redeclared with a conflicting value for `{name}`", id),
        }),
    }
}

/// Parse the value of a group `line` attribute into a [`Style`]. The
/// value is a quoted string (so `"dashed"`, not the bare contextual keyword
/// an edge body uses), and must be one of the four style names; anything else
/// is a resolve error so a typo like `line="wavy"` fails loudly instead of
/// silently rendering as a solid frame.
fn parse_line_style(value: &str, offset: usize) -> Result<Style, Error> {
    Style::from_ident(value).ok_or_else(|| Error::Resolve {
        offset,
        message: format!(
            "group `line` must be one of solid, dotted, dashed, thick; got `{value}`"
        ),
    })
}

/// Parse the value of an edge `from` / `to` attribute into an
/// [`EdgeSide`]. The value is a quoted string (so `from="left"`, not the
/// bare word), and must be one of the four page-space side names;
/// anything else is a resolve error (the same strict-attribute convention
/// as the group `line` style).
fn parse_edge_side(name: &str, value: &str, offset: usize) -> Result<EdgeSide, Error> {
    EdgeSide::from_ident(value).ok_or_else(|| Error::Resolve {
        offset,
        message: format!(
            "edge `{name}` must be one of top, bottom, left, right; got `{value}`"
        ),
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
            panic!("no node `{id}`; have {:?}", d.nodes.iter().map(|n| &n.id).collect::<Vec<_>>())
        })
    }

    #[test]
    fn basic_counts() {
        let d = ok(include_str!("../examples/infra.dgmr"));
        assert_eq!(d.direction, Direction::TopDown);
        assert_eq!(d.nodes.len(), 8);
        assert_eq!(d.edges.len(), 10);
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].title.as_deref(), Some("Kubernetes Cluster"));
        assert_eq!(d.groups[0].members, ["api1", "api2"]);
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
        let d = ok(
            "diagram top-down\n\
             a -- dotted \"sync\" color=\"#888\" --> b\n\
             c -- thick --> d\n\
             e -- \"only a label\" --> f\n",
        );
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
        let d = ok(
            "diagram top-down\n\
             a \"A\"\n\
             group \"S\"\n\
             a\n\
             end\n",
        );
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(d.groups[0].members, ["a"]);
    }

    #[test]
    fn cross_boundary_edge_does_not_relocate_node() {
        let d = ok(
            "diagram top-down\n\
             group \"S\"\n\
             a\n\
             end\n\
             b --> a\n",
        );
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
        let d = ok(
            "diagram top-down\n\
             bus \"Event Bus\"\n\
             queue \"Queue\"\n\
             group left-right \"Messaging\"\n\
             bus -- from=\"right\" to=\"left\" --> queue\n\
             end\n",
        );
        assert_eq!(node(&d, "bus").group, Some(0));
        assert_eq!(node(&d, "queue").group, Some(0));
        assert_eq!(d.groups[0].members, ["bus", "queue"]);
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        assert_eq!(d.edges.len(), 1);
    }

    #[test]
    fn edge_chain_nodes_first_seen_in_a_group_are_members() {
        let d = ok(
            "diagram top-down\n\
             group left-right \"S\"\n\
             a --> b\n\
             end\n",
        );
        assert_eq!(d.groups[0].members, ["a", "b"]);
    }

    #[test]
    fn top_level_declarations_after_a_group_do_not_un_group_its_members() {
        // Membership is decided by group bodies alone: a later top-level
        // declaration only merges label/shape/attributes, so "structure
        // first, labels later" works as well as the reverse ordering.
        let d = ok(
            "diagram top-down\n\
             group \"Messaging\"\n\
             bus --> queue\n\
             end\n\
             bus \"Event Bus\"\n\
             queue \"Queue\"\n",
        );
        assert_eq!(node(&d, "bus").group, Some(0));
        assert_eq!(node(&d, "queue").group, Some(0));
        assert_eq!(d.groups[0].members, ["bus", "queue"]);
        assert_eq!(node(&d, "bus").label, "Event Bus");
        assert_eq!(node(&d, "queue").label, "Queue");
    }

    #[test]
    fn edge_chain_in_group_body_can_reference_another_groups_node() {
        // A cross-boundary reference: `b --> a` inside B's body names `a`,
        // which already belongs to A. The edge is cross-boundary; `a` keeps
        // its group (an edge endpoint never relocates an already-placed
        // node) and only `b` becomes a member of B.
        let d = ok(
            "diagram top-down\n\
             group \"A\"\n\
             a\n\
             end\n\
             group \"B\"\n\
             b --> a\n\
             end\n",
        );
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(node(&d, "b").group, Some(1));
        assert_eq!(d.groups[0].members, ["a"]);
        assert_eq!(d.groups[1].members, ["b"]);
        assert_eq!(d.edges.len(), 1);
    }

    #[test]
    fn outer_body_chain_targeting_inner_member_keeps_nesting() {
        // The within-frame cross-boundary shape: `x --> a` written in Outer's
        // body targets `a` inside the nested Inner group; `a` must stay in
        // Inner and only `x` joins Outer.
        let d = ok(
            "diagram top-down\n\
             group \"Outer\"\n\
             x\n\
             group \"Inner\"\n\
             a\n\
             end\n\
             x --> a\n\
             end\n",
        );
        assert_eq!(node(&d, "x").group, Some(0));
        assert_eq!(node(&d, "a").group, Some(1));
        assert_eq!(d.groups[0].members, ["x"]);
        assert_eq!(d.groups[1].members, ["a"]);
    }

    #[test]
    fn nested_groups_record_containment() {
        let d = ok(
            "diagram top-down\n\
             group \"Outer\"\n\
             group \"Inner\"\n\
             a\n\
             end\n\
             end\n",
        );
        assert_eq!(d.groups.len(), 2);
        assert_eq!(node(&d, "a").group, Some(1)); // innermost
        // Containment tree: Outer contains Inner; both are top-level siblings'
        // parent links.
        assert_eq!(d.groups[0].parent, None);
        assert_eq!(d.groups[0].children, [1]);
        assert_eq!(d.groups[1].parent, Some(0));
        assert!(d.groups[1].children.is_empty());
        assert_eq!(d.groups[0].members, Vec::<String>::new());
        assert_eq!(d.groups[1].members, ["a"]);
    }

    // ---- error cases ----

    #[test]
    fn node_in_two_groups_is_an_error() {
        let (e, line) = err(
            "diagram top-down\n\
             group \"A\"\n\
             a\n\
             end\n\
             group \"B\"\n\
             a\n\
             end\n",
        );
        assert_eq!(line, 6);
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("appears in more than one group")));
    }

    #[test]
    fn conflicting_label_is_an_error() {
        let (e, line) = err("diagram top-down\na \"X\"\na \"Y\"\n");
        assert_eq!(line, 3);
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("conflicting label")));
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
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown node attribute `bogus`")));
    }

    #[test]
    fn unknown_edge_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na -- bogus=\"x\" --> b\n");
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown edge attribute `bogus`")));
    }

    #[test]
    fn duplicate_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na color=\"x\" color=\"y\"\n");
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `color`")));
    }

    #[test]
    fn group_direction_is_accepted() {
        // M4: per-group direction is now honored, not rejected.
        let d = ok(
            "diagram top-down\n\
             group left-right \"S\"\n\
             a\n\
             end\n",
        );
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        // A group with no direction inherits at layout time (stored as None).
        let d2 = ok(
            "diagram top-down\n\
             group \"T\"\n\
             b\n\
             end\n",
        );
        assert_eq!(d2.groups[0].direction, None);
    }

    #[test]
    fn missing_diagram_header_is_a_parse_error() {
        let (e, line) = err("flowchart top-down\n");
        assert_eq!(line, 1);
        assert!(matches!(e, Error::Parse { ref message, .. } if message.contains("diagram header")));
    }

    #[test]
    fn bad_direction_is_a_parse_error() {
        let (e, line) = err("diagram sideways\n");
        assert_eq!(line, 1);
        assert!(matches!(e, Error::Parse { ref message, .. } if message.contains("expected direction")));
    }

    #[test]
    fn unterminated_string_is_a_parse_error() {
        let (e, _line) = err("diagram top-down\na \"oops\n");
        assert!(matches!(e, Error::Parse { ref message, .. } if message.contains("unterminated string literal")));
    }

    #[test]
    fn unexpected_end_is_a_parse_error() {
        let (e, line) = err("diagram top-down\nend\n");
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Parse { ref message, .. } if message.contains("unexpected `end`")));
    }

    // ---- M7.5: group color/fill/line/text and text-color attributes ----

    #[test]
    fn group_style_attributes_are_parsed() {
        let d = ok(
            "diagram top-down\n\
             group \"S\" color=\"#888\" fill=\"#eef\" line=\"dashed\" text=\"#005\"\n\
             a\n\
             end\n",
        );
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
        let (e, line) = err(
            "diagram top-down\n\
             group \"S\" line=\"wavy\"\n\
             a\n\
             end\n",
        );
        assert_eq!(line, 2);
        assert!(
            matches!(e, Error::Resolve { ref message, .. }
                if message.contains("group `line` must be one of") && message.contains("wavy"))
        );
    }

    #[test]
    fn group_unknown_attribute_is_an_error() {
        let (e, line) = err(
            "diagram top-down\n\
             group \"S\" bogus=\"x\"\n\
             a\n\
             end\n",
        );
        assert_eq!(line, 2);
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("unknown group attribute `bogus`")));
    }

    #[test]
    fn group_duplicate_attribute_is_an_error() {
        let (e, _line) = err(
            "diagram top-down\n\
             group \"S\" color=\"x\" color=\"y\"\n\
             a\n\
             end\n",
        );
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `color`")));
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
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("conflicting value for `text`")));
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
        let d = ok(
            "diagram top-down\n\
             group left-right \"S\" color=\"#888\" fill=\"#eef\" line=\"dashed\" text=\"#005\"\n\
             a\n\
             b\n\
             end\n",
        );
        assert_eq!(d.groups[0].direction, Some(Direction::LeftRight));
        assert_eq!(d.groups[0].members, ["a", "b"]);
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
        assert!(
            matches!(e, Error::Resolve { ref message, .. }
                if message.contains("edge `from` must be one of") && message.contains("rught"))
        );
        let (e, line) = err("diagram top-down\na -- to=\"sideways\" --> b\n");
        assert_eq!(line, 2);
        assert!(
            matches!(e, Error::Resolve { ref message, .. }
                if message.contains("edge `to` must be one of") && message.contains("sideways"))
        );
    }

    #[test]
    fn edge_side_duplicate_attribute_is_an_error() {
        let (e, _line) = err("diagram top-down\na -- from=\"left\" from=\"right\" --> b\n");
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("duplicate attribute `from`")));
    }

    #[test]
    fn edge_side_attributes_combine_with_other_edge_attrs() {
        let d = ok(
            "diagram top-down\n\
             a -- thick \"sync\" color=\"#888\" from=\"left\" to=\"bottom\" --> b\n",
        );
        let e = &d.edges[0];
        assert_eq!(e.style, Style::Thick);
        assert_eq!(e.label.as_deref(), Some("sync"));
        assert_eq!(e.color.as_deref(), Some("#888"));
        assert_eq!(e.from_side, Some(EdgeSide::Left));
        assert_eq!(e.to_side, Some(EdgeSide::Bottom));
    }
}
