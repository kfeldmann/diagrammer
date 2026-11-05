//! Data types for the diagram grammar (see `docs/grammar.md`).
//!
//! There are two layers:
//! - the **raw** (syntactic) AST produced by the parser, preserving every
//!   declaration as written (including duplicate node occurrences);
//! - the **resolved** [`Diagram`] produced by [`crate::resolve`], with
//!   deduplicated nodes, validated attributes, and assigned group
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

    /// The grammar spelling, for error messages and the CLI summary.
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
    pub statements: Vec<RawStatement>,
}

#[derive(Debug, Clone)]
pub enum RawStatement {
    NodeList(RawNodeList),
    Group(RawGroup),
    Border(RawBorder),
    TitleBlock(RawTitleBlock),
    TextBlock(RawTextBlock),
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
}

#[derive(Debug, Clone)]
pub struct RawAttr {
    pub name: String,
    pub value: String,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct RawGroup {
    pub direction: Option<Direction>,
    pub title: Option<String>,
    pub attrs: Vec<RawAttr>,
    pub statements: Vec<RawStatement>,
}

// ---- Raw page furniture (M21.1) ----
//
// Decoration only: no block is a node, an edge endpoint, or a group member,
// and none joins the layout graph. The parser produces these; `resolve`
// validates them and hands them to `Diagram`.

/// `border attr* EOL` — the page frame.
#[derive(Debug, Clone)]
pub struct RawBorder {
    pub attrs: Vec<RawAttr>,
    pub offset: usize,
}

/// `titleblock attr* EOL field* end` — the metadata table.
///
/// The header's attrs and the body's fields share one ordered list, because
/// they share the `attr` rule: `titleblock Title="X" Company="Y"` is a
/// complete block, and a field written on the header line sits where it was
/// written. `resolve` splits the style keys (`color`, `fill`) out of it.
#[derive(Debug, Clone)]
pub struct RawTitleBlock {
    pub fields: Vec<RawAttr>,
    pub offset: usize,
}

/// `textblock anchor attr* EOL rawline* end` — free text at a page anchor.
///
/// `body` holds the raw lines verbatim (no comment stripping, no tokenizing),
/// in source order, including blank lines — which are paragraph gaps.
#[derive(Debug, Clone)]
pub struct RawTextBlock {
    pub anchor: Anchor,
    pub attrs: Vec<RawAttr>,
    pub body: Vec<String>,
    pub offset: usize,
}

// ---- Resolved (validated) diagram ----

#[derive(Debug, Clone)]
pub struct Diagram {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub groups: Vec<Group>,
    /// Page furniture (M21.1). Decoration only: these are not graph
    /// participants, so `Layout`/the renderer ignore them until M21.3's
    /// `decorate` pass. Absent/empty by default, so nothing downstream is
    /// forced to care.
    pub border: Option<Border>,
    pub title_block: Option<TitleBlock>,
    pub text_blocks: Vec<TextBlock>,
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
    /// Index into [`Diagram::groups`]; `None` means top-level.
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
/// drawn box, regardless of the diagram's or any group's layout
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
}

/// A text block's page-space anchor (M21.1). Anchors are **page-space** — the
/// visual top stays the visual top whatever the diagram's or any group's
/// direction — and they extend the `from=` / `to=` side vocabulary with the
/// four corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Top,
    UpperRight,
    Right,
    LowerRight,
    Bottom,
    LowerLeft,
    Left,
    UpperLeft,
}

impl Anchor {
    pub fn from_ident(s: &str) -> Option<Self> {
        Some(match s {
            "top" => Anchor::Top,
            "upper-right" => Anchor::UpperRight,
            "right" => Anchor::Right,
            "lower-right" => Anchor::LowerRight,
            "bottom" => Anchor::Bottom,
            "lower-left" => Anchor::LowerLeft,
            "left" => Anchor::Left,
            "upper-left" => Anchor::UpperLeft,
            _ => return None,
        })
    }

    /// The grammar spelling, for error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::UpperRight => "upper-right",
            Anchor::Right => "right",
            Anchor::LowerRight => "lower-right",
            Anchor::Bottom => "bottom",
            Anchor::LowerLeft => "lower-left",
            Anchor::Left => "left",
            Anchor::UpperLeft => "upper-left",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Group {
    pub title: Option<String>,
    /// Per-group layout direction (M4). `None` means "inherit the
    /// effective direction of the enclosing level".
    pub direction: Option<Direction>,
    /// Indices of direct child groups, in declaration order.
    pub children: Vec<usize>,
    /// Enclosing group index; `None` means this group is top-level.
    pub parent: Option<usize>,
    /// Border (line) color of the frame (M7.5). `None` falls back to the
    /// renderer's default frame stroke.
    pub color: Option<String>,
    /// Interior background fill of the frame (M7.5). `None` leaves the
    /// frame transparent (the default, so edges behind a group stay
    /// visible through it).
    pub fill: Option<String>,
    /// Border line style of the frame (M7.5): `solid`/`dotted`/`dashed`/
    /// `thick`. `None` is a plain solid frame at the default width.
    pub line: Option<Style>,
    /// Color of the frame's title text (M7.5). `None` falls back to the
    /// renderer's default title color. Applies only when `title` is present.
    pub text: Option<String>,
}

// ---- Resolved page furniture (M21.1) ----
//
// The validated form of the three decoration statements. Style values are
// carried here, not re-derived later: `resolve` is the layer that validates
// them (and that owns offsets for the errors), so the model it hands on is the
// single source of truth for what the user wrote.

/// A page border.
///
/// Presence is the whole model: the border strokes the fixed page inset, so it
/// has no geometry of its own and cannot move anything. `color=""` means "draw
/// nothing" — a validated value, not an absent statement.
#[derive(Debug, Clone)]
pub struct Border {
    /// Stroke color. `None` is the renderer's default; `Some("")` draws
    /// nothing.
    pub color: Option<String>,
    /// Stroke line style. `None` is a plain solid line.
    pub line: Option<Style>,
}

/// One title-block field: a key and its value, both verbatim.
///
/// The key is **open by design** (a title block is a metadata table whose keys
/// the user chooses — the one documented exception to "unknown attribute is an
/// error"), so it is kept exactly as written, capitalization included.
#[derive(Debug, Clone)]
pub struct Field {
    pub key: String,
    pub value: String,
}

/// A title block: its fields in declaration order, plus its style keys.
///
/// `color` and `fill` are the two **style** keys — they are not fields, so
/// `resolve` splits them out of the field list.
#[derive(Debug, Clone)]
pub struct TitleBlock {
    pub fields: Vec<Field>,
    pub color: Option<String>,
    pub fill: Option<String>,
}

/// A text block at one page anchor.
#[derive(Debug, Clone)]
pub struct TextBlock {
    pub anchor: Anchor,
    /// Raw body lines in source order, blank lines included (a blank line is
    /// a paragraph gap). Never empty and never all-whitespace.
    pub body: Vec<String>,
    pub color: Option<String>,
    pub fill: Option<String>,
    pub text: Option<String>,
    /// `width=` — a maximum content width in pixels, for word wrapping.
    /// Always positive when present.
    pub width: Option<f32>,
}
