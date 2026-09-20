//! Parser for the diagram grammar (see `docs/grammar.md`).
//!
//! Produces a raw syntactic AST ([`RawDiagram`]). Validation and
//! deduplication happen later in [`crate::resolve`].
//!
//! The parser is written as plain functions `fn(&mut &str) -> ModalResult<T>`
//! composed with winnow combinators. The top-level [`parse_diagram`] drives
//! the whole grammar via [`Parser::parse`], which yields a [`ParseError`]
//! carrying both a byte offset and accumulated context labels — these become
//! the user-facing error message and line number.

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
        && (c.is_alphanumeric() || c == '_' || c == '-') {
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
    let off = here(i, base);
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
        offset: off,
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

// ---------- subgraphs ----------

fn subgraph(i: &mut &str, base: usize) -> ModalResult<RawSubgraph> {
    lexer::ws(i)?;
    let off = here(i, base);
    keyword(i, "subgraph")?;
    lexer::ws(i)?;
    let direction = opt(direction_soft).parse_next(i)?;
    lexer::ws(i)?;
    let title = opt(lexer::string).parse_next(i)?;
    lexer::ws(i)?;
    let attrs: Vec<RawAttr> = repeat(0.., |i: &mut &str| attr(i, base)).parse_next(i)?;
    lexer::ws(i)?;
    cut_err(lexer::eol)
        .context(StrContext::Label("end of line after `subgraph` header"))
        .parse_next(i)?;

    let mut statements = Vec::new();
    loop {
        lexer::ws(i)?;
        if i.is_empty() {
            return fail_cut("expected `end` to close `subgraph`");
        }
        if opt(line_ending).parse_next(i)?.is_some() {
            continue;
        }
        if peek_keyword(i, "end") {
            keyword(i, "end")?;
            // The enclosing statement loop consumes this line's ending.
            return Ok(RawSubgraph {
                direction,
                title,
                attrs,
                statements,
                offset: off,
            });
        }
        if peek_keyword(i, "subgraph") {
            statements.push(RawStatement::Subgraph(subgraph(i, base)?));
        } else {
            statements.push(RawStatement::NodeList(nodelist(i, base)?));
        }
        lexer::ws(i)?;
        cut_err(lexer::eol)
            .context(StrContext::Label("end of line"))
            .parse_next(i)?;
    }
}

// ---------- top level ----------

fn diagram(i: &mut &str, base: usize) -> ModalResult<RawDiagram> {
    blank_lines(i)?;
    (|i: &mut &str| keyword(i, "diagram"))
        .context(StrContext::Label("diagram header"))
        .parse_next(i)?;
    lexer::ws(i)?;
    let dir_off = here(i, base);
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
            return fail_cut("unexpected `end` (no matching `subgraph`)");
        }
        if peek_keyword(i, "subgraph") {
            statements.push(RawStatement::Subgraph(subgraph(i, base)?));
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
        direction_offset: dir_off,
        statements,
    })
}
