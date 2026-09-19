//! Low-level token recognizers used by the structural parser.
//!
//! These operate directly on `&str`: whitespace/comments, line endings,
//! identifiers, and quoted strings. Structural rules (nodes, edges, subgraphs)
//! live in [`crate::parser`].

use winnow::ascii::line_ending;
use winnow::combinator::{alt, eof, opt, peek, repeat};
use winnow::error::{ContextError, ErrMode, ModalResult, StrContext};
use winnow::prelude::*;
use winnow::token::{any, one_of, take_till, take_while};

/// Skip spaces, tabs, and `#` comments. Does not consume newlines.
pub fn ws(i: &mut &str) -> ModalResult<()> {
    loop {
        let before = i.len();
        let _ = take_while(0.., |c: char| c == ' ' || c == '\t').parse_next(i)?;
        let _ = opt(comment).parse_next(i)?;
        if i.len() == before {
            break;
        }
    }
    Ok(())
}

fn comment(i: &mut &str) -> ModalResult<()> {
    "#".parse_next(i)?;
    let _ = take_till(0.., |c: char| c == '\n').parse_next(i)?;
    Ok(())
}

/// End of line: a newline (`\n` or `\r\n`) or end of input.
pub fn eol(i: &mut &str) -> ModalResult<()> {
    let _ = alt((line_ending, eof)).parse_next(i)?;
    Ok(())
}

/// Identifier: `[A-Za-z_][A-Za-z0-9_]* ( "-" [A-Za-z0-9_]+ )*`.
///
/// A hyphen is part of the id only when followed by a letter, digit, or
/// underscore, so `A-->B` lexes as `A`, `-->`, `B` (never as one id).
pub fn ident<'i>(i: &mut &'i str) -> ModalResult<String> {
    let first: char = one_of(|c: char| c.is_alphabetic() || c == '_').parse_next(i)?;
    let mid: &'i str = take_while(0.., |c: char| c.is_alphanumeric() || c == '_').parse_next(i)?;
    let mut s = String::new();
    s.push(first);
    s.push_str(mid);
    let segs: Vec<&'i str> = repeat(0.., hyphen_segment).parse_next(i)?;
    for seg in segs {
        s.push('-');
        s.push_str(seg);
    }
    Ok(s)
}

/// A `-` followed by one or more `[A-Za-z0-9_]`, matched atomically (the `-`
/// is consumed only if the segment follows, so `-->` is not eaten as part of
/// an id).
fn hyphen_segment<'i>(i: &mut &'i str) -> ModalResult<&'i str> {
    let _ = peek(|i: &mut &str| {
        "-".parse_next(i)?;
        take_while(1.., |c: char| c.is_alphanumeric() || c == '_').parse_next(i)?;
        Ok(())
    })
    .parse_next(i)?;
    "-".parse_next(i)?;
    let seg: &'i str = take_while(1.., |c: char| c.is_alphanumeric() || c == '_').parse_next(i)?;
    Ok(seg)
}

/// Double-quoted string with `\"` and `\\` escapes.
pub fn string(i: &mut &str) -> ModalResult<String> {
    "\"".parse_next(i)?;
    let mut out = String::new();
    loop {
        let chunk: &str = take_till(0.., |c: char| c == '"' || c == '\\').parse_next(i)?;
        out.push_str(chunk);
        if i.starts_with('"') {
            "\"".parse_next(i)?;
            return Ok(out);
        } else if i.starts_with('\\') {
            "\\".parse_next(i)?;
            let esc: char = any.parse_next(i)?;
            match esc {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return Err(esc_err("invalid escape sequence in string literal")),
            }
        } else {
            return Err(esc_err("unterminated string literal"));
        }
    }
}

fn esc_err(msg: &'static str) -> ErrMode<ContextError> {
    let mut e = ContextError::new();
    e.push(StrContext::Label(msg));
    ErrMode::Cut(e)
}
