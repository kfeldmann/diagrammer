//! Data types for the diagram grammar (see `docs/grammar.md`).
//!
//! There are two layers:
//! - the **raw** (syntactic) AST produced by the parser, preserving every
//!   declaration as written (including duplicate node occurrences);
//! - the **resolved** [`Diagram`] produced by [`crate::resolve`], with
//!   deduplicated nodes, validated attributes, and assigned subgraph
//!   membership.

// ---- Shared enums ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

impl Direction {
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            "top-down" => Direction::TopDown,
            "bottom-up" => Direction::BottomUp,
            "left-right" => Direction::LeftRight,
            "right-left" => Direction::RightLeft,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::TopDown => "top-down",
            Direction::BottomUp => "bottom-up",
            Direction::LeftRight => "left-right",
            Direction::RightLeft => "right-left",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Box,
    Cylinder,
}

impl Shape {
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            "box" => Shape::Box,
            "cylinder" => Shape::Cylinder,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Shape::Box => "box",
            Shape::Cylinder => "cylinder",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Solid,
    Dotted,
    Dashed,
    Thick,
}

impl Style {
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            "solid" => Style::Solid,
            "dotted" => Style::Dotted,
            "dashed" => Style::Dashed,
            "thick" => Style::Thick,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Style::Solid => "solid",
            Style::Dotted => "dotted",
            Style::Dashed => "dashed",
            Style::Thick => "thick",
        }
    }
}

// ---- Raw (syntactic) AST ----

#[derive(Debug, Clone)]
pub struct RawDiagram {
    pub direction: Direction,
    pub direction_offset: usize,
    pub statements: Vec<RawStatement>,
}

#[derive(Debug, Clone)]
pub enum RawStatement {
    NodeList(RawNodeList),
    Subgraph(RawSubgraph),
}

/// A line of the form `nodespec (edge nodespec)*`. With no edges this is a
/// single standalone node declaration; otherwise it is an edge chain whose
/// consecutive nodes form edges.
#[derive(Debug, Clone)]
pub struct RawNodeList {
    pub nodes: Vec<RawNodeDecl>,
    pub edges: Vec<RawEdgeDecl>, // len == nodes.len() - 1
}

#[derive(Debug, Clone)]
pub struct RawNodeDecl {
    pub id: String,
    pub label: Option<String>,
    pub shape: Option<Shape>,
    pub attrs: Vec<RawAttr>,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct RawEdgeDecl {
    pub style: Option<Style>,
    pub label: Option<String>,
    pub attrs: Vec<RawAttr>,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct RawAttr {
    pub name: String,
    pub value: String,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct RawSubgraph {
    pub direction: Option<Direction>,
    pub title: Option<String>,
    pub attrs: Vec<RawAttr>,
    pub statements: Vec<RawStatement>,
    pub offset: usize,
}

// ---- Resolved (validated) diagram ----

#[derive(Debug, Clone)]
pub struct Diagram {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: Vec<Subgraph>,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: Shape,
    pub color: Option<String>,
    pub fill: Option<String>,
    /// Color of the node's label text (M7.5). `None` falls back to the
    /// renderer's default ink color.
    pub text: Option<String>,
    /// Index into [`Diagram::subgraphs`]; `None` means top-level.
    pub group: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub style: Style,
    pub label: Option<String>,
    pub color: Option<String>,
    /// Color of the edge's label text (M7.5). `None` falls back to the
    /// renderer's default ink color. Applies only when `label` is present.
    pub text: Option<String>,
    /// Page-space side of the source node this edge leaves from (M11);
    /// `None` lets the layout engine choose. Honored literally — see
    /// `docs/grammar.md`.
    pub from_side: Option<EdgeSide>,
    /// Page-space side of the target node this edge enters (M11); `None`
    /// lets the layout engine choose. Honored literally.
    pub to_side: Option<EdgeSide>,
}

/// Which page-space side of a node an edge connects to (M11). Sides are
/// **page-space** (as drawn), so `top` always means the visual top of the
/// drawn box, regardless of the diagram's or any subgraph's layout
/// direction. Used by the edge `from=` / `to=` attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeSide {
    Top,
    Bottom,
    Left,
    Right,
}

impl EdgeSide {
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            "top" => EdgeSide::Top,
            "bottom" => EdgeSide::Bottom,
            "left" => EdgeSide::Left,
            "right" => EdgeSide::Right,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EdgeSide::Top => "top",
            EdgeSide::Bottom => "bottom",
            EdgeSide::Left => "left",
            EdgeSide::Right => "right",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Subgraph {
    pub title: Option<String>,
    /// Per-subgraph layout direction (M4). `None` means "inherit the
    /// effective direction of the enclosing level".
    pub direction: Option<Direction>,
    /// Direct child node ids belonging to this subgraph, in declaration
    /// order. (Deeper descendants belong to their own, inner subgraphs.)
    pub members: Vec<String>,
    /// Indices of direct child subgraphs, in declaration order.
    pub children: Vec<usize>,
    /// Enclosing subgraph index; `None` means this subgraph is top-level.
    pub parent: Option<usize>,
    /// Border (line) color of the frame (M7.5). `None` falls back to the
    /// renderer's default frame stroke.
    pub color: Option<String>,
    /// Interior background fill of the frame (M7.5). `None` leaves the
    /// frame transparent (the default, so edges behind a subgraph stay
    /// visible through it).
    pub fill: Option<String>,
    /// Border line style of the frame (M7.5): `solid`/`dotted`/`dashed`/
    /// `thick`. `None` is a plain solid frame at the default width.
    pub line: Option<Style>,
    /// Color of the frame's title text (M7.5). `None` falls back to the
    /// renderer's default title color. Applies only when `title` is present.
    pub text: Option<String>,
}
