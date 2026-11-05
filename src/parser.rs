//! Parser for the diagram grammar (see `docs/grammar.md`).
//!
//! Produces a raw syntactic AST ([`RawDiagram`]). Validation and
//! deduplication happen later in [`crate::resolve`].
//!
//! The parser is written as plain functions `fn(&mut &str) -> ModalResult<T>`
//! composed with winnow combinators. The top-level [`parse_diagram`] drives
//! the whole grammar, and winnow's parse error carries both a byte offset and
//! accumulated context labels — these become the user-facing error message
//! and line number.

use winnow::ascii::line_ending;
use winnow::combinator::{cut_err, opt, peek, preceded, repeat};
use winnow::error::{ContextError, ErrMode, ModalResult, StrContext};
use winnow::prelude::*;

use crate::ast::*;
use crate::error::Error;
use crate::lexer;

pub fn parse_diagram(source: &str) -> Result<RawDiagram, Error> {
    let base = source.as_ptr() as usize;
    let mut parser = move |i: &mut &str| diagram(i, base);
    match parser.parse(source) {
        Ok(d) => Ok(d),
        Err(pe) => {
            let message = format_context(pe.inner(), "syntax error");
            Err(Error::Parse {
                offset: pe.offset(),
                message,
            })
        }
    }
}

/// Use the first (innermost, most specific) context label, if any.
fn format_context(e: &ContextError, default: &str) -> String {
    for c in e.context() {
        if let StrContext::Label(l) = c {
            return (*l).to_string();
        }
    }
    default.to_string()
}

/// Byte offset of the current cursor within the original source.
fn here(i: &mut &str, base: usize) -> usize {
    (*i).as_ptr() as usize - base
}

/// Raise a hard (cut) error with a descriptive label at the current cursor.
fn fail_cut<T>(msg: &'static str) -> ModalResult<T> {
    let mut e = ContextError::new();
    e.push(StrContext::Label(msg));
    Err(ErrMode::Cut(e))
}

/// Skip whole blank lines: whitespace, comments, and the newlines that end
/// them. Stops at the first line that has real content.
fn blank_lines(i: &mut &str) -> ModalResult<()> {
    loop {
        lexer::ws(i)?;
        if opt(line_ending).parse_next(i)?.is_some() {
            continue;
        }
        break;
    }
    Ok(())
}

// ---------- keywords ----------

/// Match the literal `kw` only if it is a complete word (not a prefix of a
/// longer identifier). Backtracks otherwise.
fn keyword(i: &mut &str, mut kw: &'static str) -> ModalResult<()> {
    kw.parse_next(i)?;
    if let Some(c) = i.chars().next()
        && (c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ErrMode::Backtrack(ContextError::new()));
    }
    Ok(())
}

/// True if the next token is an identifier exactly equal to `kw` (without
/// consuming anything).
fn peek_keyword(i: &mut &str, kw: &str) -> bool {
    match peek(lexer::ident).parse_next(i) {
        Ok(s) => s == kw,
        Err(_) => false,
    }
}

// ---------- direction / shape / style ----------

fn direction_soft(i: &mut &str) -> ModalResult<Direction> {
    let s = peek(lexer::ident).parse_next(i)?;
    let d = Direction::from_ident(&s).ok_or_else(|| ErrMode::Backtrack(ContextError::new()))?;
    let _ = lexer::ident.parse_next(i)?;
    Ok(d)
}

fn direction_required(i: &mut &str) -> ModalResult<Direction> {
    match direction_soft(i) {
        Ok(d) => Ok(d),
        Err(ErrMode::Backtrack(_)) => {
            fail_cut("expected direction: top-down, bottom-up, left-right, or right-left")
        }
        Err(e) => Err(e),
    }
}

fn shape_kw(i: &mut &str) -> ModalResult<Shape> {
    let s = peek(lexer::ident).parse_next(i)?;
    let sh = Shape::from_ident(&s).ok_or_else(|| ErrMode::Backtrack(ContextError::new()))?;
    let _ = lexer::ident.parse_next(i)?;
    Ok(sh)
}

fn style_kw(i: &mut &str) -> ModalResult<Style> {
    let s = peek(lexer::ident).parse_next(i)?;
    let st = Style::from_ident(&s).ok_or_else(|| ErrMode::Backtrack(ContextError::new()))?;
    let _ = lexer::ident.parse_next(i)?;
    Ok(st)
}

/// A text block's anchor: a contextual keyword, matched the same way as
/// [`direction_soft`] / [`shape_kw`] / [`style_kw`] — peek the identifier,
/// validate it against the eight names, then consume. Contextuality matters:
/// `right` is an ordinary id everywhere else (`right --> left` is two nodes),
/// and only means "anchor" in the slot after `textblock`.
///
/// An unrecognized anchor is a hard error naming the eight legal values: the
/// slot is unambiguous (a text block must be anchored), so backtracking would
/// only replace a helpful message with "node declaration".
fn anchor_kw(i: &mut &str) -> ModalResult<Anchor> {
    let s = peek(lexer::ident).parse_next(i)?;
    let Some(a) = Anchor::from_ident(&s) else {
        return fail_cut(
            "expected an anchor: top, upper-right, right, lower-right, bottom, \
             lower-left, left, or upper-left",
        );
    };
    let _ = lexer::ident.parse_next(i)?;
    Ok(a)
}

// ---------- attributes ----------

fn attr(i: &mut &str, base: usize) -> ModalResult<RawAttr> {
    lexer::ws(i)?;
    let off = here(i, base);
    // Confirm "ident ws =" before committing, so a trailing identifier that
    // isn't an attribute lets the enclosing repetition stop cleanly.
    peek(|i: &mut &str| {
        let _ = lexer::ident(i)?;
        lexer::ws(i)?;
        "=".parse_next(i)?;
        Ok(())
    })
    .parse_next(i)?;
    let name = lexer::ident(i)?;
    lexer::ws(i)?;
    "=".parse_next(i)?;
    lexer::ws(i)?;
    let value = cut_err(lexer::string)
        .context(StrContext::Label("attribute value (a quoted string)"))
        .parse_next(i)?;
    Ok(RawAttr {
        name,
        value,
        offset: off,
    })
}

// ---------- nodes & edges ----------

fn nodespec(i: &mut &str, base: usize) -> ModalResult<RawNodeDecl> {
    lexer::ws(i)?;
    let off = here(i, base);
    let id = lexer::ident
        .context(StrContext::Label("node identifier"))
        .parse_next(i)?;
    let label = opt(preceded(lexer::ws, lexer::string)).parse_next(i)?;
    let shape = opt(preceded(
        lexer::ws,
        preceded(":", preceded(lexer::ws, cut_err(shape_kw))),
    ))
    .parse_next(i)?;
    let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    Ok(RawNodeDecl {
        id,
        label,
        shape,
        attrs,
        offset: off,
    })
}

fn edge(i: &mut &str, base: usize) -> ModalResult<RawEdgeDecl> {
    lexer::ws(i)?;
    "--".parse_next(i)?; // backtrack if there is no edge here
    let (style, label, attrs) = cut_err(|i: &mut &str| {
        if i.starts_with('>') {
            ">".parse_next(i)?;
            return Ok((None, None, Vec::new()));
        }
        lexer::ws(i)?;
        let style = opt(style_kw).parse_next(i)?;
        lexer::ws(i)?;
        let label = opt(lexer::string).parse_next(i)?;
        lexer::ws(i)?;
        let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
        lexer::ws(i)?;
        "-->"
            .context(StrContext::Label("edge terminator `-->`"))
            .parse_next(i)?;
        Ok((style, label, attrs))
    })
    .parse_next(i)?;
    Ok(RawEdgeDecl {
        style,
        label,
        attrs,
    })
}

fn nodelist(i: &mut &str, base: usize) -> ModalResult<RawNodeList> {
    let first = cut_err(|i: &mut &str| nodespec(i, base))
        .context(StrContext::Label("node declaration"))
        .parse_next(i)?;
    let mut nodes = vec![first];
    let mut edges = Vec::new();
    let pairs: Vec<(RawEdgeDecl, RawNodeDecl)> = repeat(0.., |i: &mut &str| {
        let e = edge(i, base)?;
        let n = cut_err(|i: &mut &str| nodespec(i, base))
            .context(StrContext::Label("node declaration"))
            .parse_next(i)?;
        Ok((e, n))
    })
    .parse_next(i)?;
    for (e, n) in pairs {
        edges.push(e);
        nodes.push(n);
    }
    Ok(RawNodeList { nodes, edges })
}

// ---------- groups ----------

fn group(i: &mut &str, base: usize) -> ModalResult<RawGroup> {
    lexer::ws(i)?;
    keyword(i, "group")?;
    lexer::ws(i)?;
    let direction = opt(direction_soft).parse_next(i)?;
    lexer::ws(i)?;
    let title = opt(lexer::string).parse_next(i)?;
    lexer::ws(i)?;
    let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    lexer::ws(i)?;
    cut_err(lexer::eol)
        .context(StrContext::Label("end of line after `group` header"))
        .parse_next(i)?;

    let mut statements = Vec::new();
    loop {
        lexer::ws(i)?;
        if i.is_empty() {
            return fail_cut("expected `end` to close `group`");
        }
        if opt(line_ending).parse_next(i)?.is_some() {
            continue;
        }
        if peek_keyword(i, "end") {
            keyword(i, "end")?;
            // The enclosing statement loop consumes this line's ending.
            return Ok(RawGroup {
                direction,
                title,
                attrs,
                statements,
            });
        }
        if peek_keyword(i, "group") {
            statements.push(RawStatement::Group(group(i, base)?));
        } else if let Some(msg) = page_furniture_in_group(i) {
            return fail_cut(msg);
        } else {
            statements.push(RawStatement::NodeList(nodelist(i, base)?));
        }
        lexer::ws(i)?;
        cut_err(lexer::eol)
            .context(StrContext::Label("end of line"))
            .parse_next(i)?;
    }
}

// ---------- page furniture (M21.1) ----------
//
// Three top-level statements that draw on the page *around* the diagram.
// They are decoration only — no block is a node, an edge endpoint, or a group
// member — so parsing them is the whole job here: `resolve` validates and
// M21.3's `decorate` pass renders.
//
// Like `nodelist`, each leaves its trailing line ending for the enclosing
// statement loop to consume (only a block's closing `end` line is treated the
// way `group`'s is).

/// Why a page-furniture keyword is illegal *here*, if the cursor sits on one.
///
/// The group-body loop dispatches `peek_keyword("group") ? group : nodelist`,
/// so without this check the three page-furniture keywords fall through to
/// `nodelist` and are parsed as **bare node declarations**: `group "G"` +
/// `textblock right` would fail with the useless `error at line 3: end of
/// line` (the anchor `right` looks like a trailing identifier), and
/// `border color="#333"` would be silently accepted as a *node* named
/// `border` with a `color` attribute. They are reserved at statement start, so
/// the only reason one appears in a body is that it was misplaced — say so.
/// This is the one place the new keywords need special-casing.
fn page_furniture_in_group(i: &mut &str) -> Option<&'static str> {
    for (kw, msg) in [
        (
            "border",
            "`border` is a page-level statement and cannot appear in a group body",
        ),
        ("titleblock", "`titleblock` cannot be a group member"),
        ("textblock", "`textblock` cannot be a group member"),
    ] {
        if peek_keyword(i, kw) {
            return Some(msg);
        }
    }
    None
}

/// `border attr* EOL` — the page frame. No body.
fn border(i: &mut &str, base: usize) -> ModalResult<RawBorder> {
    lexer::ws(i)?;
    let off = here(i, base);
    keyword(i, "border")?;
    lexer::ws(i)?;
    let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    lexer::ws(i)?;
    Ok(RawBorder { attrs, offset: off })
}

/// `titleblock attr* EOL field* end` — the metadata table.
///
/// Header attrs and body fields share the `attr` rule and accumulate into one
/// ordered list, so the one-liner form (`titleblock Title="X" Company="Y"` …
/// `end`) needs no separate code path, and a field written on the header line
/// stays where it was written.
fn titleblock(i: &mut &str, base: usize) -> ModalResult<RawTitleBlock> {
    lexer::ws(i)?;
    let off = here(i, base);
    keyword(i, "titleblock")?;
    lexer::ws(i)?;
    let mut fields: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    lexer::ws(i)?;
    cut_err(lexer::eol)
        .context(StrContext::Label("end of line after `titleblock`"))
        .parse_next(i)?;

    loop {
        lexer::ws(i)?;
        if i.is_empty() {
            return fail_cut("expected `end` to close `titleblock`");
        }
        if opt(line_ending).parse_next(i)?.is_some() {
            continue; // blank line
        }
        if peek_keyword(i, "end") {
            keyword(i, "end")?;
            // The enclosing statement loop consumes this line's ending.
            return Ok(RawTitleBlock {
                fields,
                offset: off,
            });
        }
        // `attr+` on this line. `attr` backtracks when the next thing is not
        // `ident ws =`, so a malformed field becomes a hard error naming the
        // expected shape instead of a stray node declaration.
        let line_fields: Vec<RawAttr> = repeat(1.., |i: &mut &str| attr(i, base))
            .context(StrContext::Label(
                "title block field (a `Key = \"value\"` attribute)",
            ))
            .parse_next(i)?;
        fields.extend(line_fields);
        lexer::ws(i)?;
        cut_err(lexer::eol)
            .context(StrContext::Label("end of line after a title block field"))
            .parse_next(i)?;
    }
}

/// `textblock anchor attr* EOL rawline* end` — free text at a page anchor.
///
/// The body is **raw**: each line is taken literally up to the newline (see
/// [`lexer::raw_line`]), so `#` is prose rather than a comment and `-->` or a
/// stray quote survives verbatim. The block ends at a line whose *trimmed*
/// content is exactly `end`; because `end` is a reserved word, a statement
/// consisting of `end` is unwritable anyway, so the rule is consistent with the
/// rest of the language rather than a new special case.
fn textblock(i: &mut &str, base: usize) -> ModalResult<RawTextBlock> {
    lexer::ws(i)?;
    let off = here(i, base);
    keyword(i, "textblock")?;
    lexer::ws(i)?;
    let anchor = anchor_kw(i)?;
    lexer::ws(i)?;
    let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    lexer::ws(i)?;
    cut_err(lexer::eol)
        .context(StrContext::Label("end of line after `textblock`"))
        .parse_next(i)?;

    let mut body = Vec::new();
    loop {
        if i.is_empty() {
            return fail_cut("expected `end` to close `textblock`");
        }
        let line = lexer::raw_line(i)?;
        if line.trim() == "end" {
            // The `end` text was consumed as part of the raw line; only its
            // line ending is left, and the enclosing statement loop consumes
            // that (exactly as it does after `group`'s own `end`).
            return Ok(RawTextBlock {
                anchor,
                attrs,
                body,
                offset: off,
            });
        }
        lexer::eol(i)?;
        body.push(line.to_string());
    }
}

// ---------- top level ----------

fn diagram(i: &mut &str, base: usize) -> ModalResult<RawDiagram> {
    blank_lines(i)?;
    (|i: &mut &str| keyword(i, "diagram"))
        .context(StrContext::Label("diagram header"))
        .parse_next(i)?;
    lexer::ws(i)?;
    let dir = direction_required(i)?;
    lexer::ws(i)?;
    cut_err(lexer::eol)
        .context(StrContext::Label("end of line after `diagram <direction>`"))
        .parse_next(i)?;

    let mut statements = Vec::new();
    loop {
        lexer::ws(i)?;
        if i.is_empty() {
            break;
        }
        if opt(line_ending).parse_next(i)?.is_some() {
            continue; // blank line
        }
        if peek_keyword(i, "end") {
            return fail_cut("unexpected `end` (no matching `group`)");
        }
        if peek_keyword(i, "group") {
            statements.push(RawStatement::Group(group(i, base)?));
        } else if peek_keyword(i, "border") {
            statements.push(RawStatement::Border(border(i, base)?));
        } else if peek_keyword(i, "titleblock") {
            statements.push(RawStatement::TitleBlock(titleblock(i, base)?));
        } else if peek_keyword(i, "textblock") {
            statements.push(RawStatement::TextBlock(textblock(i, base)?));
        } else {
            statements.push(RawStatement::NodeList(nodelist(i, base)?));
        }
        lexer::ws(i)?;
        cut_err(lexer::eol)
            .context(StrContext::Label("end of line"))
            .parse_next(i)?;
    }
    Ok(RawDiagram {
        direction: dir,
        statements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::line_of;

    fn parse(src: &str) -> RawDiagram {
        parse_diagram(src).unwrap_or_else(|e| panic!("should parse: {e:?}\n{src}"))
    }

    /// The parse error and the 1-based line it points at.
    fn parse_err(src: &str) -> (Error, usize) {
        match parse_diagram(src) {
            Ok(_) => panic!("expected a parse error\n{src}"),
            Err(e) => {
                let line = match &e {
                    Error::Parse { offset, .. } => line_of(src, *offset),
                    other => panic!("expected a parse error, got {other:?}"),
                };
                (e, line)
            }
        }
    }

    fn statements(d: &RawDiagram) -> &[RawStatement] {
        &d.statements
    }

    // ---- page furniture: header forms (M21.1) ----

    #[test]
    fn border_parses_alone_and_with_attrs() {
        let d = parse("diagram top-down\nborder\n");
        match &statements(&d)[0] {
            RawStatement::Border(b) => assert!(b.attrs.is_empty()),
            other => panic!("expected a border, got {other:?}"),
        }
        let d = parse("diagram top-down\nborder color=\"#333\" line=\"dashed\"\n");
        match &statements(&d)[0] {
            RawStatement::Border(b) => {
                assert_eq!(b.attrs.len(), 2);
                assert_eq!(b.attrs[0].name, "color");
                assert_eq!(b.attrs[1].value, "dashed");
            }
            other => panic!("expected a border, got {other:?}"),
        }
    }

    #[test]
    fn titleblock_parses_fields_and_the_one_liner_form() {
        let d = parse("diagram top-down\ntitleblock\n  Title = \"X\"\n  Company = \"Y\"\nend\n");
        match &statements(&d)[0] {
            RawStatement::TitleBlock(t) => {
                let names: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).collect();
                assert_eq!(names, ["Title", "Company"]);
            }
            other => panic!("expected a title block, got {other:?}"),
        }
        // Header and body share the `attr` rule, so the one-liner needs no
        // separate code path — and the header's fields stay where written.
        let d = parse("diagram top-down\ntitleblock Title=\"X\" Company=\"Y\"\nend\n");
        match &statements(&d)[0] {
            RawStatement::TitleBlock(t) => {
                let names: Vec<&str> = t.fields.iter().map(|f| f.name.as_str()).collect();
                assert_eq!(names, ["Title", "Company"]);
            }
            other => panic!("expected a title block, got {other:?}"),
        }
        // An empty body is legal at parse time; emptiness is resolve's call.
        let d = parse("diagram top-down\ntitleblock\nend\n");
        assert!(matches!(&statements(&d)[0], RawStatement::TitleBlock(t) if t.fields.is_empty()));
    }

    #[test]
    fn textblock_parses_anchor_attrs_and_body() {
        let d =
            parse("diagram top-down\ntextblock upper-left width=\"300\"\n  hello\n  world\nend\n");
        match &statements(&d)[0] {
            RawStatement::TextBlock(t) => {
                assert_eq!(t.anchor, Anchor::UpperLeft);
                assert_eq!(t.attrs.len(), 1);
                assert_eq!(t.attrs[0].name, "width");
                assert_eq!(t.body, ["  hello", "  world"]);
            }
            other => panic!("expected a text block, got {other:?}"),
        }
    }

    #[test]
    fn every_anchor_parses() {
        for (src, want) in [
            ("top", Anchor::Top),
            ("upper-right", Anchor::UpperRight),
            ("right", Anchor::Right),
            ("lower-right", Anchor::LowerRight),
            ("bottom", Anchor::Bottom),
            ("lower-left", Anchor::LowerLeft),
            ("left", Anchor::Left),
            ("upper-left", Anchor::UpperLeft),
        ] {
            let d = parse(&format!("diagram top-down\ntextblock {src}\n  x\nend\n"));
            match &statements(&d)[0] {
                RawStatement::TextBlock(t) => assert_eq!(t.anchor, want, "anchor {src}"),
                other => panic!("expected a text block, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_anchor_is_a_cut_error_naming_the_eight() {
        let (e, line) = parse_err("diagram top-down\ntextblock middle\n  x\nend\n");
        assert_eq!(line, 2);
        let Error::Parse { message, .. } = &e else {
            panic!("expected a parse error");
        };
        assert!(message.contains("expected an anchor"), "{message}");
        for name in [
            "top",
            "upper-right",
            "right",
            "lower-right",
            "bottom",
            "lower-left",
            "left",
            "upper-left",
        ] {
            assert!(message.contains(name), "{message} names {name}");
        }
    }

    #[test]
    fn unterminated_blocks_name_the_block() {
        let (e, _) = parse_err("diagram top-down\ntextblock right\n  x\n");
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("expected `end` to close `textblock`")),
            "{e:?}"
        );
        let (e, _) = parse_err("diagram top-down\ntitleblock\n  Title = \"X\"\n");
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("expected `end` to close `titleblock`")),
            "{e:?}"
        );
    }

    #[test]
    fn malformed_title_block_field_is_a_cut_error() {
        // A body line that is not `Key = "value"` — the `attr` repetition
        // backtracks, so the failure must be cut into a message about fields.
        let (e, line) = parse_err("diagram top-down\ntitleblock\n  just some prose\nend\n");
        assert_eq!(line, 3);
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("title block field")),
            "{e:?}"
        );
    }

    // ---- the raw body: prose is prose ----

    #[test]
    fn text_block_body_is_verbatim() {
        // `#` is not a comment here, `-->` is not an arrow, a quoted string is
        // not a string, and leading whitespace survives (the *renderer*
        // trims; the parser must not).
        let src = "diagram top-down\n\
             textblock right\n  \
             # not a comment\n  \
             a --> b --> c\n  \
             a quote \" left open\n  \
             trailing spaces   \n  \
             **bold** and _italic_ and ---\n\
             end\n";
        let d = parse(src);
        match &statements(&d)[0] {
            RawStatement::TextBlock(t) => {
                assert_eq!(
                    t.body,
                    [
                        "  # not a comment",
                        "  a --> b --> c",
                        "  a quote \" left open",
                        "  trailing spaces   ",
                        "  **bold** and _italic_ and ---",
                    ]
                );
            }
            other => panic!("expected a text block, got {other:?}"),
        }
    }

    #[test]
    fn text_block_body_keeps_blank_lines() {
        let d = parse("diagram top-down\ntextblock right\n  a\n\n  b\nend\n");
        match &statements(&d)[0] {
            RawStatement::TextBlock(t) => assert_eq!(t.body, ["  a", "", "  b"]),
            other => panic!("expected a text block, got {other:?}"),
        }
    }

    #[test]
    fn text_block_ends_only_on_a_trimmed_end_line() {
        // `end` inside prose stays prose; the block closes on the trimmed
        // `end` line, and the `end` line's newline is left for the enclosing
        // loop (which is why a following statement parses).
        let src = "diagram top-down\n\
             textblock right\n  \
             the end is nigh\n  \
             end of the line\n  \
             end\n\
             a\n";
        let d = parse(src);
        match &statements(&d)[0] {
            RawStatement::TextBlock(t) => {
                assert_eq!(t.body, ["  the end is nigh", "  end of the line"])
            }
            other => panic!("expected a text block, got {other:?}"),
        }
        assert!(matches!(&statements(&d)[1], RawStatement::NodeList(_)));
    }

    #[test]
    fn text_block_body_at_eof_without_trailing_newline() {
        let d = parse("diagram top-down\ntextblock right\n  x\nend");
        assert!(matches!(&statements(&d)[0], RawStatement::TextBlock(t) if t.body == ["  x"]));
    }

    #[test]
    fn crlf_text_block_body_has_no_carriage_returns() {
        // `eol` accepts `\r\n`, so the raw line must not smuggle the CR into
        // the body (it would reach the SVG `<text>` and `text::measure`).
        let d = parse("diagram top-down\r\ntextblock right\r\n  a\r\n  b\r\nend\r\n");
        match &statements(&d)[0] {
            RawStatement::TextBlock(t) => assert_eq!(t.body, ["  a", "  b"]),
            other => panic!("expected a text block, got {other:?}"),
        }
    }

    // ---- the group-body trap ----

    #[test]
    fn page_furniture_in_a_group_body_is_rejected_by_name() {
        // Without the explicit check these fall through to `nodelist` and are
        // parsed as bare node declarations (`border color="#333"` would be
        // silently accepted as a *node* named `border`).
        let (e, line) = parse_err("diagram top-down\ngroup \"G\"\ntextblock right\nend\n");
        assert_eq!(line, 3);
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("`textblock` cannot be a group member")),
            "{e:?}"
        );
        let (e, line) = parse_err("diagram top-down\ngroup \"G\"\ntitleblock\nend\n");
        assert_eq!(line, 3);
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("`titleblock` cannot be a group member")),
            "{e:?}"
        );
        let (e, line) = parse_err("diagram top-down\ngroup \"G\"\nborder color=\"#333\"\nend\n");
        assert_eq!(line, 3);
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("`border` is a page-level statement")),
            "{e:?}"
        );
    }

    #[test]
    fn page_furniture_in_a_nested_group_body_is_rejected_too() {
        let (e, line) =
            parse_err("diagram top-down\ngroup \"Outer\"\ngroup \"Inner\"\nborder\nend\nend\n");
        assert_eq!(line, 4);
        assert!(
            matches!(&e, Error::Parse { message, .. }
                if message.contains("page-level")),
            "{e:?}"
        );
    }

    // ---- contextuality: the keywords stay usable as ids ----

    #[test]
    fn the_new_keywords_are_only_reserved_at_statement_start() {
        // `a --> border` is an edge to a node named `border`; `right --> left`
        // is two nodes named `right` and `left` (an anchor only means anchor
        // in the slot after `textblock`).
        let d = parse("diagram top-down\na --> border\nright --> left\n");
        match &statements(&d)[0] {
            RawStatement::NodeList(nl) => assert_eq!(nl.nodes[1].id, "border"),
            other => panic!("expected a node list, got {other:?}"),
        }
        match &statements(&d)[1] {
            RawStatement::NodeList(nl) => {
                assert_eq!(nl.nodes[0].id, "right");
                assert_eq!(nl.nodes[1].id, "left");
            }
            other => panic!("expected a node list, got {other:?}"),
        }
        // And a prefix of a longer id is a node, not a statement.
        let d = parse("diagram top-down\nborderline\ntextblocks\n");
        assert_eq!(statements(&d).len(), 2);
        assert!(
            statements(&d)
                .iter()
                .all(|s| matches!(s, RawStatement::NodeList(_)))
        );
    }

    #[test]
    fn page_furniture_mixes_with_nodes_and_groups_in_any_order() {
        let d = parse(
            "diagram top-down\n\
             web\n\
             border\n\
             group \"G\"\n\
             api\n\
             end\n\
             textblock bottom\n  hi\nend\n\
             titleblock\n  Title = \"X\"\nend\n",
        );
        let kinds: Vec<&str> = statements(&d)
            .iter()
            .map(|s| match s {
                RawStatement::NodeList(_) => "nodes",
                RawStatement::Group(_) => "group",
                RawStatement::Border(_) => "border",
                RawStatement::TitleBlock(_) => "titleblock",
                RawStatement::TextBlock(_) => "textblock",
            })
            .collect();
        assert_eq!(
            kinds,
            ["nodes", "border", "group", "textblock", "titleblock"]
        );
    }
}
