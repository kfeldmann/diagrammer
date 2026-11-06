//! Page furniture (M21.3): the post-routing `decorate` pass.
//!
//! The three page-furniture statements — `border`, `titleblock`, and
//! `textblock` — draw on the page *around* the diagram, so they are placed
//! **after** routing has finished: [`crate::layout::layout`] calls
//! [`decorate`] once `assemble` has produced a complete [`Layout`], and never
//! before. No block is a node, an edge endpoint, or a group member, so none of
//! them ever joins the layout graph, the obstacle model, or the routing
//! vocabulary — a route can never see one.
//!
//! # The page inset
//!
//! The page reserves a fixed [`PAGE_INSET`] between the canvas edge and the
//! border rectangle. It exists whenever *any* furniture is present, so adding
//! or removing a `border` statement changes no coordinate — the statement only
//! strokes a rectangle that was already reserved. (With no furniture at all the
//! pass is a no-op, which is what keeps every pre-M21 rendering
//! byte-identical.)
//!
//! # Band reservation
//!
//! Each occupied anchor turns its block's extent into an extra inset between
//! the content's bounding box and the border rectangle: `top`/`bottom`/
//! `left`/`right` add to their own side, a corner takes from both, and a
//! left/right band forces a minimum border *height* (a top/bottom band a
//! minimum *width*) so a small diagram still gets a coherent frame. The canvas
//! grows and every piece of content is translated uniformly, so no relative
//! geometry changes.
//!
//! The title block is reserved first: it owns the lower-right corner of the
//! page, so a `right` or `bottom` text block occupies the span that remains
//! *beside* it rather than the full side. (`resolve` already rejects a
//! `textblock lower-right` alongside a `titleblock`.)

use crate::ast::{Anchor, Diagram, TextBlock, TitleBlock};
use crate::layout::{BlockLine, BlockRect, Layout, MARGIN, Rect, TextRun};
use crate::text::{self, Weight};

/// Distance from the canvas edge to the page border rectangle (M21.3). It is
/// reserved whether or not a `border` statement is present, so `border` only
/// strokes the rectangle the inset already set aside.
pub const PAGE_INSET: f32 = 28.0;

/// Clearance between a page block and the border rectangle (M21.3). The band a
/// block reserves is its own extent plus this gap, so a block never touches the
/// border it sits inside.
pub const BLOCK_GAP: f32 = 12.0;

/// Padding inside a text block's frame, each side.
pub const BLOCK_PAD: f32 = 10.0;

/// Text-block body font size.
pub const TEXT_SIZE: f32 = 13.0;

/// Title-block body font size.
pub const TITLEBLOCK_SIZE: f32 = 12.0;

/// Font size of a `Title` field — one size up, the single piece of hierarchy
/// that makes the table read as a title block rather than a list.
pub const TITLEBLOCK_TITLE_SIZE: f32 = 14.0;

/// Padding inside the title-block table, each side.
pub const TB_PAD: f32 = 6.0;

/// Gap between the title block's label and value columns.
pub const TB_COL_GAP: f32 = 12.0;

/// Vertical padding inside one title-block row.
pub const TB_ROW_PAD: f32 = 4.0;

/// Vertical space a blank source line (a paragraph gap) consumes — half a body
/// line, so a paragraph break reads as a break rather than a whole empty line.
const PARAGRAPH_GAP: f32 = 0.5 * TEXT_SIZE * text::LINE_FACTOR;

/// Minimum drawn width of a `---` rule, so a block holding only a rule still
/// draws something visible.
const MIN_RULE_W: f32 = 48.0;

/// The `decorate` pass (M21.3): place the page furniture against the finished
/// canvas and grow the canvas to hold it.
///
/// A no-op — leaving every coordinate and every `Layout` field untouched —
/// when the diagram uses none of the three statements, which is what makes
/// every pre-M21 rendering byte-identical.
pub(crate) fn decorate(diagram: &Diagram, layout: &mut Layout) {
    if diagram.border.is_none() && diagram.title_block.is_none() && diagram.text_blocks.is_empty() {
        return;
    }

    // ---- 1. Size every block (content only; positions come after the bands).
    // An empty title block has nothing to draw, so it reserves no table band —
    // the page frame is still reserved, giving a border-only page.
    let table = diagram
        .title_block
        .as_ref()
        .map(title_table)
        .filter(|t| !t.rows.is_empty());
    let blocks: Vec<TextLayout> = diagram.text_blocks.iter().map(text_layout).collect();

    // ---- 2. Band reservation --------------------------------------------
    // Distance from the content's bounding box to the border rectangle, per
    // side. `MARGIN` alone when a side holds nothing: the content keeps its
    // existing clearance guarantee against the border it now sits inside.
    //
    // The title block is reserved first, because it owns the lower-right
    // corner: its table's bottom and right edges snap to the border rectangle,
    // so its full extent joins the bottom and right bands.
    let mut left = MARGIN;
    let mut right = MARGIN;
    let mut top = MARGIN;
    let mut bottom = MARGIN;
    if let Some(t) = &table {
        right = right.max(MARGIN + t.w);
        bottom = bottom.max(MARGIN + t.h);
    }
    for b in &blocks {
        // A block sits `BLOCK_GAP` inside the border, and content keeps
        // `MARGIN` clear of it.
        let vertical = MARGIN + b.h + BLOCK_GAP;
        let horizontal = MARGIN + b.w + BLOCK_GAP;
        let (on_top, on_bottom, on_left, on_right) = anchor_sides(b.anchor);
        if on_top {
            top = top.max(vertical);
        }
        if on_bottom {
            bottom = bottom.max(vertical);
        }
        if on_left {
            left = left.max(horizontal);
        }
        if on_right {
            right = right.max(horizontal);
        }
    }

    // A side band spans the border's full height (or width), so it forces a
    // coherent minimum on the perpendicular extent: a two-node diagram with a
    // tall `right` block still gets a frame that contains it. Everything that
    // shares a band stacks in it, each with a `BLOCK_GAP`: an edge block is
    // centered *between* the corner blocks of its column (or the title block),
    // so the column's minimum is the sum of its parts. Left and right columns,
    // top and bottom rows, are computed separately (blocks on opposite sides of
    // the page do not stack).
    //
    // A band's minimum is the sum of its parts plus a `BLOCK_GAP` each and a
    // gap on the band's two edges. A left/right column measures each part's
    // *height* (a corner block contributes its height to the column it sits
    // in), a top/bottom row each part's *width*.
    let band_min = |parts: &[f32]| -> f32 {
        if parts.is_empty() {
            0.0
        } else {
            parts.iter().sum::<f32>() + (parts.len() as f32 + 1.0) * BLOCK_GAP
        }
    };
    let band = |on_side: fn((bool, bool, bool, bool)) -> bool,
                extent: fn(&TextLayout) -> f32,
                tb: Option<f32>|
     -> f32 {
        let mut parts: Vec<f32> = blocks
            .iter()
            .filter(|b| on_side(anchor_sides(b.anchor)))
            .map(&extent)
            .collect();
        parts.extend(tb);
        band_min(&parts)
    };
    let tb_h = table.as_ref().map(|t| t.h);
    let tb_w = table.as_ref().map(|t| t.w);
    // The left/right columns measure height; the top/bottom rows measure width.
    let min_h = band(|s| s.2, |b| b.h, None).max(band(|s| s.3, |b| b.h, tb_h));
    let min_w = band(|s| s.0, |b| b.w, None).max(band(|s| s.1, |b| b.w, tb_w));

    let (w, h) = (layout.width, layout.height);
    let mut x0 = -left;
    let mut x1 = w + right;
    let mut y0 = -top;
    let mut y1 = h + bottom;
    let cur_h = y1 - y0;
    if cur_h < min_h {
        let extra = (min_h - cur_h) / 2.0;
        y0 -= extra;
        y1 += extra;
    }
    let cur_w = x1 - x0;
    if cur_w < min_w {
        let extra = (min_w - cur_w) / 2.0;
        x0 -= extra;
        x1 += extra;
    }

    // ---- 3. Translate the finished content into the reserved page ---------
    let dx = PAGE_INSET - x0;
    let dy = PAGE_INSET - y0;
    for n in &mut layout.nodes {
        n.x += dx;
        n.y += dy;
    }
    for g in &mut layout.groups {
        g.x += dx;
        g.y += dy;
    }
    for e in &mut layout.edges {
        for p in &mut e.points {
            p.0 += dx;
            p.1 += dy;
        }
        if let Some(p) = e.label_at.as_mut() {
            p.0 += dx;
            p.1 += dy;
        }
    }

    let border = Rect {
        x: PAGE_INSET,
        y: PAGE_INSET,
        w: x1 - x0,
        h: y1 - y0,
    };
    layout.width = border.w + 2.0 * PAGE_INSET;
    layout.height = border.h + 2.0 * PAGE_INSET;

    // ---- 4. Place the blocks against the border rectangle ------------------
    // `color=""` draws nothing, so a suppressed border is `None` — the inset is
    // still reserved, there is just nothing to stroke.
    layout.border = match &diagram.border {
        Some(b) if b.color.as_deref() != Some("") => Some(border),
        _ => None,
    };
    // The table snaps its bottom and right edges to the border rectangle.
    let tb_rect = table.as_ref().map(|t| Rect {
        x: border.x + border.w - t.w,
        y: border.y + border.h - t.h,
        w: t.w,
        h: t.h,
    });
    layout.title_block = tb_rect;

    // The corner blocks' rects, so an edge block can center on the span they
    // leave free (a `left` block must clear `upper-left` and `lower-left`).
    // Corners are placed first — they are pinned, so they need no free span.
    let content = Rect { x: dx, y: dy, w, h };
    let mut corners: Vec<(Anchor, Rect)> = Vec::new();
    let mut placed: Vec<Option<BlockRect>> = vec![None; blocks.len()];
    for (i, b) in blocks.iter().enumerate() {
        if is_corner(b.anchor) {
            let r = place_text_block(b.clone(), &border, tb_rect.as_ref(), &[], &content);
            corners.push((
                blocks[i].anchor,
                Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                },
            ));
            placed[i] = Some(r);
        }
    }
    for (i, b) in blocks.iter().enumerate() {
        if placed[i].is_none() {
            placed[i] = Some(place_text_block(
                b.clone(),
                &border,
                tb_rect.as_ref(),
                &corners,
                &content,
            ));
        }
    }
    layout.text_blocks = placed.into_iter().map(Option::unwrap).collect();
}

/// Is this one of the four corner anchors (as opposed to an edge anchor)?
fn is_corner(anchor: Anchor) -> bool {
    matches!(
        anchor,
        Anchor::UpperLeft | Anchor::UpperRight | Anchor::LowerLeft | Anchor::LowerRight
    )
}

/// Which of the four sides an anchor's band comes out of: `(top, bottom, left,
/// right)`. A corner takes from both of its sides; an edge anchor from one.
fn anchor_sides(anchor: Anchor) -> (bool, bool, bool, bool) {
    match anchor {
        Anchor::Top => (true, false, false, false),
        Anchor::Bottom => (false, true, false, false),
        Anchor::Left => (false, false, true, false),
        Anchor::Right => (false, false, false, true),
        Anchor::UpperLeft => (true, false, true, false),
        Anchor::UpperRight => (true, false, false, true),
        Anchor::LowerLeft => (false, true, true, false),
        Anchor::LowerRight => (false, true, false, true),
    }
}

/// A text block's content, sized but not yet positioned.
#[derive(Clone)]
struct TextLayout {
    anchor: Anchor,
    w: f32,
    h: f32,
    /// The block's lines in *block-local* coordinates (x from the frame's left
    /// edge, y from its top edge). [`place_text_block`] offsets them into page
    /// coordinates.
    lines: Vec<BlockLine>,
}

/// Lay out one text block's body: parse the inline markup, wrap to `width=`,
/// and measure. Lines are laid out top to bottom; each text line takes one
/// [`text::line_height`], a `---` rule one line height, and a blank line (a
/// paragraph gap) [`PARAGRAPH_GAP`].
fn text_layout(b: &TextBlock) -> TextLayout {
    let lh = text::line_height(TEXT_SIZE);
    // Leading and trailing blank lines would only pad the frame, so they are
    // dropped (a paragraph break only means something *between* paragraphs).
    let body: Vec<&str> = b.body.iter().map(|l| l.trim()).collect();
    let first = body.iter().position(|l| !l.is_empty()).unwrap_or(0);
    let last = body.iter().rposition(|l| !l.is_empty()).unwrap_or(0);
    let body = &body[first..=last];

    let mut lines: Vec<BlockLine> = Vec::new();
    let mut y = BLOCK_PAD;
    let mut content_w: f32 = 0.0;
    let mut has_rule = false;
    for line in body {
        if line.is_empty() {
            lines.push(BlockLine::Gap);
            y += PARAGRAPH_GAP;
        } else if *line == "---" {
            has_rule = true;
            lines.push(BlockLine::Rule {
                y: y + lh / 2.0,
                x: BLOCK_PAD,
                w: 0.0,
            });
            y += lh;
        } else {
            for runs in wrap_runs(parse_inline(line), b.width) {
                let line_w: f32 = runs.iter().map(|r| r.width).sum();
                content_w = content_w.max(line_w);
                lines.push(BlockLine::Text {
                    y: y + lh / 2.0,
                    x: BLOCK_PAD,
                    runs,
                });
                y += lh;
            }
        }
    }
    if has_rule {
        content_w = content_w.max(MIN_RULE_W);
    }
    for line in &mut lines {
        if let BlockLine::Rule { w, .. } = line {
            *w = content_w;
        }
    }
    TextLayout {
        anchor: b.anchor,
        w: content_w + 2.0 * BLOCK_PAD,
        h: y + BLOCK_PAD,
        lines,
    }
}

/// Position a sized block against the border rectangle and return the
/// renderer's [`BlockRect`] (frame rect plus lines in page coordinates).
///
/// Each anchor owns a *slot*. A corner anchor pins to its corner of the page.
/// An edge anchor centers on what remains of its band: the content's bounding
/// box, except where a corner block in the same band takes part of the span
/// (an `upper-left` block eats the top of the left column, the title block the
/// bottom of the right column) — then it centers on the gap between them. That
/// is what keeps `left` clear of `upper-left`/`lower-left` on a short page.
fn place_text_block(
    b: TextLayout,
    border: &Rect,
    title_block: Option<&Rect>,
    corners: &[(Anchor, Rect)],
    content: &Rect,
) -> BlockRect {
    let (bw, bh) = (b.w, b.h);
    // The inner rectangle a block may occupy: the border minus `BLOCK_GAP`.
    let (ix0, iy0) = (border.x + BLOCK_GAP, border.y + BLOCK_GAP);
    let (ix1, iy1) = (
        border.x + border.w - BLOCK_GAP,
        border.y + border.h - BLOCK_GAP,
    );
    // The vertical span left free in a column by the corner blocks (and the
    // title block on the right), and the horizontal span left free in a row by
    // the corner blocks (and the title block on the bottom).
    let free_v = |anchors: &[Anchor], right_side: bool| -> (f32, f32) {
        let mut lo = iy0;
        let mut hi = iy1;
        for (a, r) in corners {
            if anchors.contains(a) {
                if *a == Anchor::UpperLeft || *a == Anchor::UpperRight {
                    lo = lo.max(r.y + r.h + BLOCK_GAP);
                } else {
                    hi = hi.min(r.y - BLOCK_GAP);
                }
            }
        }
        if right_side && let Some(t) = title_block {
            hi = hi.min(t.y - BLOCK_GAP);
        }
        (lo, hi)
    };
    let free_h = |anchors: &[Anchor], bottom_row: bool| -> (f32, f32) {
        let mut lo = ix0;
        let mut hi = ix1;
        for (a, r) in corners {
            if anchors.contains(a) {
                if *a == Anchor::UpperLeft || *a == Anchor::LowerLeft {
                    lo = lo.max(r.x + r.w + BLOCK_GAP);
                } else {
                    hi = hi.min(r.x - BLOCK_GAP);
                }
            }
        }
        if bottom_row && let Some(t) = title_block {
            hi = hi.min(t.x - BLOCK_GAP);
        }
        (lo, hi)
    };
    // Center the block on `want` (the content's center along that axis), but
    // clamp it into the free span `[lo, hi]` so it can never collide with a
    // corner block. When the span is too small to hold the block (a page that
    // was not grown for this corner combination), pin to the span's start so
    // the block stays inside the border.
    let fit = |want: f32, lo: f32, hi: f32, size: f32| -> f32 {
        let start = want - size / 2.0;
        if hi - lo >= size {
            start.max(lo).min(hi - size)
        } else {
            lo
        }
    };
    let (x, y) = match b.anchor {
        Anchor::Top => {
            let (lo, hi) = free_h(&[Anchor::UpperLeft, Anchor::UpperRight], false);
            (fit(content.x + content.w / 2.0, lo, hi, bw), iy0)
        }
        Anchor::Bottom => {
            let (lo, hi) = free_h(&[Anchor::LowerLeft, Anchor::LowerRight], true);
            (fit(content.x + content.w / 2.0, lo, hi, bw), iy1 - bh)
        }
        Anchor::Left => {
            let (lo, hi) = free_v(&[Anchor::UpperLeft, Anchor::LowerLeft], false);
            (ix0, fit(content.y + content.h / 2.0, lo, hi, bh))
        }
        Anchor::Right => {
            let (lo, hi) = free_v(&[Anchor::UpperRight, Anchor::LowerRight], true);
            (ix1 - bw, fit(content.y + content.h / 2.0, lo, hi, bh))
        }
        // Corner anchors pin to their corner of the page.
        Anchor::UpperLeft => (ix0, iy0),
        Anchor::UpperRight => (ix1 - bw, iy0),
        Anchor::LowerLeft => (ix0, iy1 - bh),
        Anchor::LowerRight => (ix1 - bw, iy1 - bh),
    };
    let mut lines = b.lines;
    for line in &mut lines {
        match line {
            BlockLine::Gap => {}
            BlockLine::Rule { x: lx, y: ly, .. } | BlockLine::Text { x: lx, y: ly, .. } => {
                *lx += x;
                *ly += y;
            }
        }
    }
    BlockRect {
        x,
        y,
        w: bw,
        h: bh,
        lines,
        line_height: text::line_height(TEXT_SIZE),
    }
}

// ---- Title block ---------------------------------------------------------

/// One row of the title block's table.
pub struct TableRow {
    pub key: String,
    pub value: String,
    /// The row's font size (`Title` is one size up).
    pub size: f32,
    /// The row's height (the taller of key and value, plus padding).
    pub h: f32,
}

/// The title block's table geometry. Shared by [`decorate`] (which sizes the
/// band and the snapped rect from it) and the renderer (which draws the grid
/// and the text from it), so the two can never disagree about where a row
/// starts.
pub struct Table {
    pub rows: Vec<TableRow>,
    /// Widest key.
    pub label_w: f32,
    pub w: f32,
    pub h: f32,
}

/// Lay out a title block's table: row height = the taller of key and value
/// plus padding, the label column the widest key, the value column the widest
/// value.
///
/// Keys are drawn **bold** (they are the table's labels) and are therefore
/// measured with the Bold table, so the label column is as wide as the glyphs
/// the renderer will actually draw.
pub fn title_table(tb: &TitleBlock) -> Table {
    let mut rows = Vec::with_capacity(tb.fields.len());
    let mut label_w = 0.0_f32;
    let mut value_w = 0.0_f32;
    for f in &tb.fields {
        let size = if f.key == "Title" {
            TITLEBLOCK_TITLE_SIZE
        } else {
            TITLEBLOCK_SIZE
        };
        let k = text::measure_run(&f.key, size, Weight::Bold);
        let v = text::measure(&f.value, size);
        label_w = label_w.max(k.width);
        value_w = value_w.max(v.width);
        rows.push(TableRow {
            key: f.key.clone(),
            value: f.value.clone(),
            size,
            h: k.height.max(v.height) + 2.0 * TB_ROW_PAD,
        });
    }
    let h = rows.iter().map(|r| r.h).sum::<f32>() + 2.0 * TB_PAD;
    let w = 2.0 * TB_PAD + label_w + TB_COL_GAP + value_w;
    Table {
        rows,
        label_w,
        w,
        h,
    }
}

// ---- Inline markup -------------------------------------------------------

/// The inline markup a text block understands (a small Markdown subset,
/// applied per line — an inline run never spans a line break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// `**` — bold.
    Bold,
    /// `***` — bold and italic.
    BoldStar,
    /// `*` — italic.
    Star,
    /// `_` — italic.
    Under,
}

impl Mark {
    /// The source spelling, for an unmatched (literal) marker.
    fn as_str(self) -> &'static str {
        match self {
            Mark::Bold => "**",
            Mark::BoldStar => "***",
            Mark::Star => "*",
            Mark::Under => "_",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Ch(char),
    /// A markup marker, and whether it may *open* a run here.
    Delim(Mark, bool),
}

/// Parse one line of a text block into styled runs.
///
/// Prose is not code: the language is strict about syntax and forgiving about
/// typography, so an **unbalanced** marker is literal text rather than an
/// error. A marker opens a run only at a **word boundary** (the preceding
/// character is not alphanumeric), which is what keeps `snake_case_name`
/// literal. `\*`, `\_`, and `\\` are escapes, so a literal marker is always
/// writable.
fn parse_inline(line: &str) -> Vec<TextRun> {
    let cs: Vec<char> = line.chars().collect();
    let at = |i: usize| -> Option<char> { cs.get(i).copied() };
    // "Opens a run only at a word boundary": the character before the marker
    // is not alphanumeric (start of line counts as a boundary) and the first
    // character after it is not whitespace.
    let can_open = |i: usize, len: usize| -> bool {
        let boundary = match i.checked_sub(1).and_then(at) {
            None => true,
            Some(c) => !c.is_alphanumeric(),
        };
        boundary && at(i + len).is_some_and(|c| !c.is_whitespace())
    };

    // ---- tokenize ----
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' {
            match at(i + 1) {
                Some(next @ ('*' | '_' | '\\')) => {
                    toks.push(Tok::Ch(next));
                    i += 2;
                }
                // Any other backslash is literal prose.
                _ => {
                    toks.push(Tok::Ch('\\'));
                    i += 1;
                }
            }
            continue;
        }
        if c == '*' {
            if at(i + 1) == Some('*') && at(i + 2) == Some('*') {
                toks.push(Tok::Delim(Mark::BoldStar, can_open(i, 3)));
                i += 3;
            } else if at(i + 1) == Some('*') {
                toks.push(Tok::Delim(Mark::Bold, can_open(i, 2)));
                i += 2;
            } else {
                toks.push(Tok::Delim(Mark::Star, can_open(i, 1)));
                i += 1;
            }
            continue;
        }
        if c == '_' {
            toks.push(Tok::Delim(Mark::Under, can_open(i, 1)));
            i += 1;
            continue;
        }
        toks.push(Tok::Ch(c));
        i += 1;
    }

    // ---- match openers with closers ----
    // A marker closes only the run on top of the stack: crossing markers is
    // not nesting, so `**a _b**` leaves every marker literal (the documented
    // "an unbalanced marker is literal text" rule).
    let mut matched = vec![false; toks.len()];
    let mut stack: Vec<(usize, Mark)> = Vec::new();
    for (idx, tok) in toks.iter().enumerate() {
        if let Tok::Delim(mark, opens) = *tok {
            if stack.last().map(|&(_, m)| m) == Some(mark) {
                let (open, _) = stack.pop().expect("stack is non-empty");
                matched[open] = true;
                matched[idx] = true;
            } else if opens {
                stack.push((idx, mark));
            }
        }
    }

    // ---- emit runs ----
    let mut runs: Vec<TextRun> = Vec::new();
    let (mut bold, mut italic) = (false, false);
    for (idx, tok) in toks.iter().enumerate() {
        match *tok {
            Tok::Ch(c) => push_run(&mut runs, &c.to_string(), bold, italic),
            Tok::Delim(mark, _) => {
                if matched[idx] {
                    match mark {
                        Mark::Bold => bold = !bold,
                        Mark::Star | Mark::Under => italic = !italic,
                        Mark::BoldStar => {
                            bold = !bold;
                            italic = !italic;
                        }
                    }
                } else {
                    push_run(&mut runs, mark.as_str(), bold, italic);
                }
            }
        }
    }
    runs
}

/// Append `text` to `runs`, merging into the previous run when the style
/// matches (so a line reads as a handful of runs, not one per character).
fn push_run(runs: &mut Vec<TextRun>, text: &str, bold: bool, italic: bool) {
    if text.is_empty() {
        return;
    }
    let weight = if bold { Weight::Bold } else { Weight::Regular };
    if let Some(last) = runs.last_mut()
        && last.weight == weight
        && last.italic == italic
    {
        last.text.push_str(text);
        last.width = text::measure_run(&last.text, TEXT_SIZE, weight).width;
        return;
    }
    runs.push(TextRun {
        text: text.to_string(),
        weight,
        italic,
        width: text::measure_run(text, TEXT_SIZE, weight).width,
    });
}

/// Split one line's runs into wrapped lines. With no `width` the line is left
/// whole ("text blocks do not wrap unless `width=` is given"); with one, words
/// are packed greedily. A word wider than `width` gets its own line rather than
/// being broken — prose, not code.
///
/// The runs are **flattened to characters first**, then grouped into words at
/// spaces. Splitting each *run* on spaces instead would cut a styled word away
/// from the punctuation that abuts it: `_italic_,` arrives as the runs
/// `italic` + `,`, and treating those as two words would render "italic ,".
fn wrap_runs(runs: Vec<TextRun>, width: Option<f32>) -> Vec<Vec<TextRun>> {
    /// One styled fragment of a word (`_italic_,` is two: `italic` and `,`).
    struct Piece {
        text: String,
        bold: bool,
        italic: bool,
        w: f32,
    }
    /// A word: one or more pieces, plus whether a space preceded it.
    struct Word {
        pieces: Vec<Piece>,
        w: f32,
    }

    // Flatten to styled characters.
    let chars: Vec<(char, bool, bool)> = runs
        .iter()
        .flat_map(|r| {
            let bold = r.weight == Weight::Bold;
            r.text.chars().map(move |c| (c, bold, r.italic))
        })
        .collect();

    // Group into words at spaces.
    let mut words: Vec<Word> = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    let push_char = |pieces: &mut Vec<Piece>, c: char, bold: bool, italic: bool| {
        if let Some(last) = pieces.last_mut()
            && last.bold == bold
            && last.italic == italic
        {
            last.text.push(c);
            return;
        }
        pieces.push(Piece {
            text: c.to_string(),
            bold,
            italic,
            w: 0.0,
        });
    };
    let flush = |pieces: &mut Vec<Piece>, words: &mut Vec<Word>| {
        if pieces.is_empty() {
            return;
        }
        let weight = |p: &Piece| {
            if p.bold {
                Weight::Bold
            } else {
                Weight::Regular
            }
        };
        for p in pieces.iter_mut() {
            p.w = text::measure_run(&p.text, TEXT_SIZE, weight(p)).width;
        }
        let w = pieces.iter().map(|p| p.w).sum();
        words.push(Word {
            pieces: std::mem::take(pieces),
            w,
        });
    };
    for (c, bold, italic) in chars {
        if c == ' ' {
            flush(&mut pieces, &mut words);
        } else {
            push_char(&mut pieces, c, bold, italic);
        }
    }
    flush(&mut pieces, &mut words);
    if words.is_empty() {
        return vec![Vec::new()];
    }

    // Greedy packing.
    let space_w = text::measure_run(" ", TEXT_SIZE, Weight::Regular).width;
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_w = 0.0_f32;
    for (i, word) in words.iter().enumerate() {
        let needed = if cur.is_empty() {
            word.w
        } else {
            space_w + word.w
        };
        if width.is_some_and(|max| !cur.is_empty() && cur_w + needed > max) {
            lines.push(std::mem::take(&mut cur));
            cur_w = word.w;
        } else {
            cur_w += needed;
        }
        cur.push(i);
    }
    lines.push(cur);

    // Re-join each line's words into runs, merging neighbours of one style and
    // inserting a plain space between words.
    lines
        .into_iter()
        .map(|idxs| {
            let mut out: Vec<TextRun> = Vec::new();
            for (k, i) in idxs.iter().enumerate() {
                if k > 0 {
                    push_run(&mut out, " ", false, false);
                }
                for p in &words[*i].pieces {
                    push_run(&mut out, &p.text, p.bold, p.italic);
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Border, TextBlock};

    /// A text block with the given anchor/body/width, no styling.
    fn block(anchor: Anchor, body: &[&str], width: Option<f32>) -> TextBlock {
        TextBlock {
            anchor,
            body: body.iter().map(|s| s.to_string()).collect(),
            color: None,
            fill: None,
            text: None,
            width,
        }
    }

    /// The concatenated text of a line's runs.
    fn line_text(line: &BlockLine) -> String {
        match line {
            BlockLine::Text { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            BlockLine::Rule { .. } => "---".to_string(),
            BlockLine::Gap => String::new(),
        }
    }

    /// The `(weight, italic)` style of every run in a line, in order.
    fn line_styles(line: &BlockLine) -> Vec<(Weight, bool)> {
        match line {
            BlockLine::Text { runs, .. } => runs.iter().map(|r| (r.weight, r.italic)).collect(),
            _ => Vec::new(),
        }
    }

    // ---- inline markup ---------------------------------------------------

    #[test]
    fn plain_text_is_one_regular_run() {
        let runs = parse_inline("hello world");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hello world");
        assert_eq!(runs[0].weight, Weight::Regular);
        assert!(!runs[0].italic);
    }

    #[test]
    fn bold_and_italic_markers_set_style_and_vanish() {
        let runs = parse_inline("a **bold** b _it_ c *it2* d");
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "a bold b it c it2 d");
        let bold: Vec<&str> = runs
            .iter()
            .filter(|r| r.weight == Weight::Bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, vec!["bold"]);
        let italic: Vec<&str> = runs
            .iter()
            .filter(|r| r.italic)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(italic, vec!["it", "it2"]);
    }

    #[test]
    fn bold_italic_nesting_carries_both() {
        let runs = parse_inline("x **bold _and italic_** y");
        let both: Vec<&str> = runs
            .iter()
            .filter(|r| r.weight == Weight::Bold && r.italic)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(both, vec!["and italic"]);
        // The `***` spelling nests in one marker.
        let runs = parse_inline("a ***both*** b");
        assert!(
            runs.iter()
                .any(|r| r.weight == Weight::Bold && r.italic && r.text == "both"),
            "{runs:?}"
        );
    }

    #[test]
    fn unbalanced_marker_is_literal_text() {
        let runs = parse_inline("a ** b");
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "a ** b");
        assert!(runs.iter().all(|r| r.weight == Weight::Regular));
        // Crossing markers is not nesting: `**` cannot close an open `_`, so
        // neither pair matches and both stay literal.
        let runs = parse_inline("**a _b**");
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "**a _b**");
        assert!(
            runs.iter()
                .all(|r| r.weight == Weight::Regular && !r.italic)
        );
    }

    #[test]
    fn underscore_opens_only_at_a_word_boundary() {
        // `snake_case_name` has no word-boundary underscore, so it is literal.
        let runs = parse_inline("snake_case_name");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "snake_case_name");
        assert!(!runs[0].italic);
        // An underscore at a boundary does open a run.
        let runs = parse_inline("a _b_ c");
        assert!(runs.iter().any(|r| r.italic && r.text == "b"));
    }

    #[test]
    fn escapes_produce_literal_markers() {
        let runs = parse_inline(r"a \* b \_ c \\ d");
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, r"a * b _ c \ d");
        assert!(
            runs.iter()
                .all(|r| r.weight == Weight::Regular && !r.italic)
        );
    }

    #[test]
    fn bold_run_measures_wider_than_regular() {
        let runs = parse_inline("**measure me**");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].weight, Weight::Bold);
        let regular = text::measure("measure me", TEXT_SIZE).width;
        assert!(
            runs[0].width > regular,
            "bold run should use the bold table: {} vs {regular}",
            runs[0].width
        );
    }

    // ---- block layout ----------------------------------------------------

    #[test]
    fn rule_and_paragraph_gap_layout() {
        let t = text_layout(&block(Anchor::Left, &["first", "", "---", "last"], None));
        let kinds: Vec<&str> = t
            .lines
            .iter()
            .map(|l| match l {
                BlockLine::Gap => "gap",
                BlockLine::Rule { .. } => "rule",
                BlockLine::Text { .. } => "text",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "gap", "rule", "text"]);
        // The rule spans the block's content width.
        let rule_w = t
            .lines
            .iter()
            .find_map(|l| match l {
                BlockLine::Rule { w, .. } => Some(*w),
                _ => None,
            })
            .unwrap();
        assert!(rule_w >= MIN_RULE_W, "rule too short: {rule_w}");
    }

    #[test]
    fn wrapping_splits_a_long_line_and_not_a_short_one() {
        let long = "one two three four five six seven eight nine ten eleven twelve";
        let unwrapped = text_layout(&block(Anchor::Top, &[long], None));
        let wrapped = text_layout(&block(Anchor::Top, &[long], Some(120.0)));
        let text_lines = |t: &TextLayout| {
            t.lines
                .iter()
                .filter(|l| matches!(l, BlockLine::Text { .. }))
                .count()
        };
        assert_eq!(text_lines(&unwrapped), 1, "no width ⇒ no wrapping");
        assert!(text_lines(&wrapped) > 1, "width ⇒ wraps");
        // The wrapped text still reads the same (words, in order).
        let joined: Vec<String> = wrapped
            .lines
            .iter()
            .filter(|l| matches!(l, BlockLine::Text { .. }))
            .map(line_text)
            .collect();
        assert_eq!(joined.join(" "), long);
    }

    #[test]
    fn wrapped_styled_word_keeps_its_adjacent_punctuation() {
        // `_italic_,` must not become "italic ,": punctuation abutting a styled
        // word is part of the same word, even though it is a different run.
        let t = text_layout(&block(Anchor::Top, &["a _italic_, b"], Some(200.0)));
        let text: String = t.lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        assert!(text.contains("italic,"), "punctuation detached: {text:?}");
        assert!(!text.contains("italic ,"), "space before comma: {text:?}");
    }

    #[test]
    fn block_size_includes_padding_and_leading_blank_lines_are_dropped() {
        let t = text_layout(&block(Anchor::Top, &["", "  hello  ", ""], None));
        // One text line only.
        assert_eq!(
            t.lines
                .iter()
                .filter(|l| matches!(l, BlockLine::Text { .. }))
                .count(),
            1
        );
        assert!(!t.lines.iter().any(|l| matches!(l, BlockLine::Gap)));
        assert!(t.h >= 2.0 * BLOCK_PAD);
        assert!(t.w > 2.0 * BLOCK_PAD);
    }

    // ---- band reservation / placement -----------------------------------

    /// Build a `Layout` of the given content size with no furniture, run the
    /// decorate pass, and return the result.
    fn decorated(w: f32, h: f32, d: &Diagram) -> Layout {
        let mut l = Layout {
            nodes: Vec::new(),
            edges: Vec::new(),
            groups: Vec::new(),
            width: w,
            height: h,
            border: None,
            title_block: None,
            text_blocks: Vec::new(),
        };
        decorate(d, &mut l);
        l
    }

    fn diagram(border: bool, blocks: Vec<TextBlock>) -> Diagram {
        Diagram {
            direction: crate::ast::Direction::TopDown,
            nodes: Vec::new(),
            edges: Vec::new(),
            groups: Vec::new(),
            border: border.then_some(Border {
                color: None,
                line: None,
            }),
            title_block: None,
            text_blocks: blocks,
        }
    }

    #[test]
    fn no_furniture_is_a_no_op() {
        let d = diagram(false, Vec::new());
        let l = decorated(300.0, 200.0, &d);
        assert_eq!((l.width, l.height), (300.0, 200.0));
        assert!(l.border.is_none());
        assert!(l.title_block.is_none());
        assert!(l.text_blocks.is_empty());
    }

    #[test]
    fn border_is_inset_from_the_canvas_and_content_is_translated_uniformly() {
        let d = diagram(true, Vec::new());
        let l = decorated(300.0, 200.0, &d);
        let b = l.border.expect("border");
        assert!((b.x - PAGE_INSET).abs() < 1e-3);
        assert!((b.y - PAGE_INSET).abs() < 1e-3);
        assert!((l.width - (b.w + 2.0 * PAGE_INSET)).abs() < 1e-3);
        // The content's MARGIN guarantee: the border clears it by MARGIN.
        assert!((b.w - (300.0 + 2.0 * MARGIN)).abs() < 1e-3, "{}", b.w);
        assert!((b.h - (200.0 + 2.0 * MARGIN)).abs() < 1e-3, "{}", b.h);
    }

    #[test]
    fn suppressed_border_reserves_the_inset_but_draws_nothing() {
        let mut d = diagram(true, Vec::new());
        d.border = Some(Border {
            color: Some(String::new()),
            line: None,
        });
        let l = decorated(300.0, 200.0, &d);
        assert!(l.border.is_none(), "color=\"\" draws no border");
        // The inset is still reserved: the canvas grew the same way.
        assert!(l.width > 300.0);
    }

    #[test]
    fn border_presence_does_not_move_any_block() {
        // The decoupling invariant: adding `border` changes no coordinate.
        let block = block(Anchor::Right, &["hello"], None);
        let with = decorated(300.0, 200.0, &diagram(true, vec![block.clone()]));
        let without = decorated(300.0, 200.0, &diagram(false, vec![block]));
        let a = &with.text_blocks[0];
        let b = &without.text_blocks[0];
        assert_eq!((a.x, a.y, a.w, a.h), (b.x, b.y, b.w, b.h));
    }

    #[test]
    fn text_block_sits_inside_the_border_and_clear_of_the_content() {
        let d = diagram(true, vec![block(Anchor::Right, &["hi"], None)]);
        let l = decorated(300.0, 200.0, &d);
        let b = l.border.expect("border");
        let t = &l.text_blocks[0];
        assert!(t.x >= b.x && t.x + t.w <= b.x + b.w);
        assert!(t.y >= b.y && t.y + t.h <= b.y + b.h);
        // The block's left edge is at least MARGIN past the content's right.
        assert!(t.x >= 300.0 + MARGIN - 1e-2, "block at {t:?} over content");
    }

    #[test]
    fn title_block_snaps_to_the_lower_right_of_the_border() {
        let mut d = diagram(true, Vec::new());
        d.title_block = Some(TitleBlock {
            fields: vec![
                crate::ast::Field {
                    key: "Title".into(),
                    value: "T".into(),
                },
                crate::ast::Field {
                    key: "Company".into(),
                    value: "C".into(),
                },
            ],
            color: None,
            fill: None,
        });
        let l = decorated(300.0, 200.0, &d);
        let b = l.border.expect("border");
        let t = l.title_block.expect("title block");
        assert!((t.x + t.w - (b.x + b.w)).abs() < 1e-3, "right edges snap");
        assert!((t.y + t.h - (b.y + b.h)).abs() < 1e-3, "bottom edges snap");
    }

    #[test]
    fn title_field_is_one_size_up() {
        let tb = TitleBlock {
            fields: vec![
                crate::ast::Field {
                    key: "Title".into(),
                    value: "A".into(),
                },
                crate::ast::Field {
                    key: "Version".into(),
                    value: "1".into(),
                },
            ],
            color: None,
            fill: None,
        };
        let table = title_table(&tb);
        assert_eq!(table.rows[0].size, TITLEBLOCK_TITLE_SIZE);
        assert_eq!(table.rows[1].size, TITLEBLOCK_SIZE);
        assert!(table.rows[0].h > table.rows[1].h, "the title row is taller");
    }

    #[test]
    fn right_block_stacks_above_the_title_block() {
        let mut d = diagram(true, vec![block(Anchor::Right, &["note"], None)]);
        d.title_block = Some(TitleBlock {
            fields: vec![crate::ast::Field {
                key: "Title".into(),
                value: "T".into(),
            }],
            color: None,
            fill: None,
        });
        let l = decorated(300.0, 200.0, &d);
        let t = &l.text_blocks[0];
        let tb = l.title_block.expect("title block");
        assert!(
            t.y + t.h <= tb.y + 1e-2,
            "right block {} should sit above the title block {}",
            t.y + t.h,
            tb.y
        );
    }

    #[test]
    fn small_diagram_gets_a_frame_tall_enough_for_a_side_block() {
        // A tiny content box with a tall `right` block still gets a border
        // that contains the block.
        let body: Vec<String> = (0..12).map(|i| format!("line {i}")).collect();
        let mut d = diagram(true, Vec::new());
        d.text_blocks = vec![TextBlock {
            anchor: Anchor::Right,
            body,
            color: None,
            fill: None,
            text: None,
            width: None,
        }];
        let l = decorated(80.0, 40.0, &d);
        let b = l.border.expect("border");
        let t = &l.text_blocks[0];
        assert!(t.y >= b.y && t.y + t.h <= b.y + b.h, "block {t:?} in {b:?}");
    }

    #[test]
    fn lines_are_positioned_in_page_coordinates() {
        let d = diagram(true, vec![block(Anchor::Top, &["one", "two"], None)]);
        let l = decorated(300.0, 200.0, &d);
        let t = &l.text_blocks[0];
        let ys: Vec<f32> = t
            .lines
            .iter()
            .filter_map(|line| match line {
                BlockLine::Text { y, .. } => Some(*y),
                _ => None,
            })
            .collect();
        assert_eq!(ys.len(), 2);
        assert!(ys[0] > t.y && ys[1] > ys[0]);
        assert!(ys[1] < t.y + t.h);
        // A text line's x starts inside the frame's padding.
        if let BlockLine::Text { x, .. } = &t.lines[0] {
            assert!((*x - (t.x + BLOCK_PAD)).abs() < 1e-3);
        } else {
            panic!("first line should be text");
        }
    }

    #[test]
    fn styled_runs_survive_the_block_layout() {
        let d = diagram(true, vec![block(Anchor::Left, &["a **b** _c_"], None)]);
        let l = decorated(300.0, 200.0, &d);
        let line = l.text_blocks[0]
            .lines
            .iter()
            .find(|l| matches!(l, BlockLine::Text { .. }))
            .expect("a text line");
        assert_eq!(line_text(line), "a b c");
        let styles = line_styles(line);
        assert!(styles.contains(&(Weight::Bold, false)));
        assert!(styles.contains(&(Weight::Regular, true)));
    }

    /// Do two rects overlap?
    fn overlaps(a: &BlockRect, b: &BlockRect) -> bool {
        a.x < b.x + b.w - 1e-2
            && b.x < a.x + a.w - 1e-2
            && a.y < b.y + b.h - 1e-2
            && b.y < a.y + a.h - 1e-2
    }

    #[test]
    fn a_side_block_clears_the_corner_blocks_of_its_column() {
        // `left` is centered between `upper-left` and `lower-left`; on a page
        // too short for all three to fit side by side, the band minimum grows
        // the border so they never collide.
        let tall = |anchor, n: usize| TextBlock {
            anchor,
            body: (0..n).map(|i| format!("line {i}")).collect(),
            color: None,
            fill: None,
            text: None,
            width: None,
        };
        let mut d = diagram(true, Vec::new());
        d.text_blocks = vec![
            tall(Anchor::Left, 10),
            tall(Anchor::UpperLeft, 1),
            tall(Anchor::LowerLeft, 1),
        ];
        let l = decorated(120.0, 60.0, &d);
        let [left, ul, ll] = [&l.text_blocks[0], &l.text_blocks[1], &l.text_blocks[2]];
        assert!(!overlaps(left, ul), "left {left:?} vs upper-left {ul:?}");
        assert!(!overlaps(left, ll), "left {left:?} vs lower-left {ll:?}");
        assert!(!overlaps(ul, ll));
        // All inside the border.
        let b = l.border.expect("border");
        for t in &l.text_blocks {
            assert!(t.x >= b.x && t.y >= b.y);
            assert!(t.x + t.w <= b.x + b.w && t.y + t.h <= b.y + b.h);
        }
    }

    #[test]
    fn all_eight_anchors_never_collide() {
        // The full set on a small page: every block stays inside the border and
        // no two overlap.
        let b = |anchor| block(anchor, &["block"], None);
        let mut d = diagram(true, Vec::new());
        d.text_blocks = vec![
            b(Anchor::UpperLeft),
            b(Anchor::Top),
            b(Anchor::UpperRight),
            b(Anchor::Left),
            b(Anchor::Right),
            b(Anchor::LowerLeft),
            b(Anchor::Bottom),
            b(Anchor::LowerRight),
        ];
        d.title_block = Some(TitleBlock {
            fields: vec![crate::ast::Field {
                key: "Title".into(),
                value: "T".into(),
            }],
            color: None,
            fill: None,
        });
        // `lower-right` conflicts with a title block at resolve; drop it.
        d.text_blocks.pop();
        let l = decorated(120.0, 60.0, &d);
        let border = l.border.expect("border");
        for (i, t) in l.text_blocks.iter().enumerate() {
            assert!(
                t.x >= border.x - 1e-2
                    && t.y >= border.y - 1e-2
                    && t.x + t.w <= border.x + border.w + 1e-2
                    && t.y + t.h <= border.y + border.h + 1e-2,
                "block {i} {t:?} outside border {border:?}"
            );
            for (j, u) in l.text_blocks.iter().enumerate().skip(i + 1) {
                assert!(!overlaps(t, u), "blocks {i} and {j} overlap: {t:?} {u:?}");
            }
            if let Some(tb) = l.title_block {
                let tr = BlockRect {
                    x: tb.x,
                    y: tb.y,
                    w: tb.w,
                    h: tb.h,
                    lines: Vec::new(),
                    line_height: 0.0,
                };
                assert!(!overlaps(t, &tr), "block {i} overlaps the title block");
            }
        }
    }

    #[test]
    fn empty_title_block_reserves_the_page_but_draws_no_table() {
        let mut d = diagram(false, Vec::new());
        d.title_block = Some(TitleBlock {
            fields: Vec::new(),
            color: None,
            fill: None,
        });
        let l = decorated(300.0, 200.0, &d);
        assert!(l.title_block.is_none(), "no rows ⇒ no table");
        // The canvas still grew (the inset is reserved).
        assert!(l.width > 300.0);
    }
}
