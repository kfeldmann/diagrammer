//! Semantic validation: turns a raw syntactic [`RawDiagram`] into a validated
//! [`Diagram`].
//!
//! Responsibilities:
//! - Deduplicate node declarations, enforcing label / shape / attribute
//!   consistency across redeclarations.
//! - Assign subgraph membership. A node may be placed in at most one subgraph;
//!   placing it in two is an error. Declaring a node at top level and later
//!   referencing it inside a `subgraph` body moves it into that group (this is
//!   the common pattern).
//! - Validate attributes strictly: unknown attributes and duplicate
//!   attributes within a single declaration are errors.
//! - Record the subgraph containment tree (each subgraph's `parent` and
//!   `children`) and carry each subgraph's optional per-subgraph `direction`
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
    /// Current group: `None` = top level, `Some(idx)` = a subgraph.
    membership: Option<usize>,
    /// Distinct subgraphs this node has been placed in. Length >= 2 is an
    /// error ("appears in more than one subgraph").
    placed_subgraphs: Vec<usize>,
}

struct SubgraphInfo {
    title: Option<String>,
    direction: Option<Direction>,
    /// Enclosing subgraph index; `None` = top level. Captured when the
    /// subgraph is opened so the resolved model can express nesting.
    parent: Option<usize>,
    #[allow(dead_code)]
    offset: usize,
}

struct Ctx {
    nodes: Vec<NodeInfo>,
    id_index: HashMap<String, usize>,
    edges: Vec<Edge>,
    subgraphs: Vec<SubgraphInfo>,
}

pub fn resolve(raw: &RawDiagram) -> Result<Diagram, Error> {
    let mut ctx = Ctx {
        nodes: Vec::new(),
        id_index: HashMap::new(),
        edges: Vec::new(),
        subgraphs: Vec::new(),
    };
    for stmt in &raw.statements {
        ctx.process_statement(stmt, None)?;
    }

    let subgraphs: Vec<Subgraph> = ctx
        .subgraphs
        .iter()
        .enumerate()
        .map(|(idx, sg)| {
            let members: Vec<String> = ctx
                .nodes
                .iter()
                .filter(|n| n.membership == Some(idx))
                .map(|n| n.id.clone())
                .collect();
            Subgraph {
                title: sg.title.clone(),
                direction: sg.direction,
                members,
                children: Vec::new(),
                parent: sg.parent,
            }
        })
        .collect();
    // Populate each subgraph's direct children (by parent link) in index order.
    let mut subgraphs = subgraphs;
    for (idx, sg) in ctx.subgraphs.iter().enumerate() {
        if let Some(p) = sg.parent {
            subgraphs[p].children.push(idx);
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
            group: n.membership,
        })
        .collect();

    Ok(Diagram {
        direction: raw.direction,
        nodes,
        edges: ctx.edges,
        subgraphs,
    })
}

impl Ctx {
    fn process_statement(&mut self, stmt: &RawStatement, group: Option<usize>) -> Result<(), Error> {
        match stmt {
            RawStatement::NodeList(nl) => self.process_nodelist(nl, group),
            RawStatement::Subgraph(sg) => self.process_subgraph(sg, group),
        }
    }

    fn process_subgraph(&mut self, sg: &RawSubgraph, parent: Option<usize>) -> Result<(), Error> {
        // Per-subgraph direction is now honored by layout (M4); the grammar
        // already validates the direction token, so we only carry it through.
        // No subgraph attributes are defined for v0, so any attribute is unknown.
        validate_attrs(&sg.attrs, &[], "subgraph")?;
        let idx = self.subgraphs.len();
        self.subgraphs.push(SubgraphInfo {
            title: sg.title.clone(),
            direction: sg.direction,
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
            validate_attrs(&e.attrs, &["color"], "edge")?;
            let mut color = None;
            for attr in &e.attrs {
                if attr.name == "color" {
                    color = Some(attr.value.clone());
                }
            }
            self.edges.push(Edge {
                from,
                to,
                style: e.style.unwrap_or(Style::Solid),
                label: e.label.clone(),
                color,
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
        validate_attrs(&occ.attrs, &["color", "fill"], "node")?;
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
                    _ => unreachable!("validate_attrs ensures only known node attributes"),
                }
            }
            // Only a standalone occurrence (not an edge endpoint) places a
            // node; edge endpoints are pure references and never move a node.
            if is_standalone {
                match group {
                    Some(g) => {
                        if !info.placed_subgraphs.contains(&g) {
                            info.placed_subgraphs.push(g);
                            if info.placed_subgraphs.len() >= 2 {
                                return Err(Error::Resolve {
                                    offset: occ.offset,
                                    message: format!(
                                        "node `{}` appears in more than one subgraph",
                                        occ.id
                                    ),
                                });
                            }
                        }
                        info.membership = Some(g);
                    }
                    None => info.membership = None,
                }
            }
        } else {
            // New node: implicitly declared in the current group.
            let mut color = None;
            let mut fill = None;
            for attr in &occ.attrs {
                match attr.name.as_str() {
                    "color" => color = Some(attr.value.clone()),
                    "fill" => fill = Some(attr.value.clone()),
                    _ => unreachable!("validate_attrs ensures only known node attributes"),
                }
            }
            let mut placed_subgraphs = Vec::new();
            if let Some(g) = group {
                placed_subgraphs.push(g);
            }
            let idx = self.nodes.len();
            self.nodes.push(NodeInfo {
                id: occ.id.clone(),
                label: occ.label.clone(),
                shape: occ.shape,
                color,
                fill,
                membership: group,
                placed_subgraphs,
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

/// Validate that every attribute name is allowed for this position and that
/// no attribute name repeats within a single declaration.
fn validate_attrs(attrs: &[RawAttr], allowed: &[&str], pos_name: &str) -> Result<(), Error> {
    let mut seen: Vec<String> = Vec::new();
    for attr in attrs {
        if !allowed.iter().any(|a| *a == attr.name.as_str()) {
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
        let d = ok(include_str!("../examples/infra.mmd"));
        assert_eq!(d.direction, Direction::TopDown);
        assert_eq!(d.nodes.len(), 8);
        assert_eq!(d.edges.len(), 10);
        assert_eq!(d.subgraphs.len(), 1);
        assert_eq!(d.subgraphs[0].title.as_deref(), Some("Kubernetes Cluster"));
        assert_eq!(d.subgraphs[0].members, ["api1", "api2"]);
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
    fn move_top_level_node_into_subgraph() {
        let d = ok(
            "diagram top-down\n\
             a \"A\"\n\
             subgraph \"S\"\n\
             a\n\
             end\n",
        );
        assert_eq!(node(&d, "a").group, Some(0));
        assert_eq!(d.subgraphs[0].members, ["a"]);
    }

    #[test]
    fn cross_boundary_edge_does_not_relocate_node() {
        let d = ok(
            "diagram top-down\n\
             subgraph \"S\"\n\
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
    fn nested_subgraphs_record_containment() {
        let d = ok(
            "diagram top-down\n\
             subgraph \"Outer\"\n\
             subgraph \"Inner\"\n\
             a\n\
             end\n\
             end\n",
        );
        assert_eq!(d.subgraphs.len(), 2);
        assert_eq!(node(&d, "a").group, Some(1)); // innermost
        // Containment tree: Outer contains Inner; both are top-level siblings'
        // parent links.
        assert_eq!(d.subgraphs[0].parent, None);
        assert_eq!(d.subgraphs[0].children, [1]);
        assert_eq!(d.subgraphs[1].parent, Some(0));
        assert!(d.subgraphs[1].children.is_empty());
        assert_eq!(d.subgraphs[0].members, Vec::<String>::new());
        assert_eq!(d.subgraphs[1].members, ["a"]);
    }

    // ---- error cases ----

    #[test]
    fn node_in_two_subgraphs_is_an_error() {
        let (e, line) = err(
            "diagram top-down\n\
             subgraph \"A\"\n\
             a\n\
             end\n\
             subgraph \"B\"\n\
             a\n\
             end\n",
        );
        assert_eq!(line, 6);
        assert!(matches!(e, Error::Resolve { ref message, .. } if message.contains("appears in more than one subgraph")));
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
    fn subgraph_direction_is_accepted() {
        // M4: per-subgraph direction is now honored, not rejected.
        let d = ok(
            "diagram top-down\n\
             subgraph left-right \"S\"\n\
             a\n\
             end\n",
        );
        assert_eq!(d.subgraphs.len(), 1);
        assert_eq!(d.subgraphs[0].direction, Some(Direction::LeftRight));
        // A subgraph with no direction inherits at layout time (stored as None).
        let d2 = ok(
            "diagram top-down\n\
             subgraph \"T\"\n\
             b\n\
             end\n",
        );
        assert_eq!(d2.subgraphs[0].direction, None);
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
}
