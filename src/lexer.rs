//! Low-level token recognizers used by the structural parser.
//!
//! These operate directly on `&str`: whitespace/comments, line endings,
//! identifiers, and quoted strings. Structural rules (nodes, edges, groups)
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
    peek(|i: &mut &str| {
        "-".parse_next(i)?;
        take_while(1.., |c: char| c.is_alphanumeric() || c == '_').parse_next(i)?;
        Ok(())
    })
    .parse_next(i)?;
    "-".parse_next(i)?;
    let seg: &'i str = take_while(1.., |c: char| c.is_alphanumeric() || c == '_').parse_next(i)?;
    Ok(seg)
}

/// A **raw body line**: everything up to (but not including) the newline,
/// taken literally — no comment stripping, no tokenizing, no escape
/// processing.
///
/// This is the one recognizer the page-furniture bodies need (M21.1): a
/// `textblock` body is prose, so `#` there is literal text rather than a
/// comment, and `-->` or a stray quote must survive verbatim. The newline
/// itself is left for [`eol`], like every other rule here.
///
/// A trailing `\r` is trimmed: `take_till` stops only at `\n`, so the CR of a
/// `\r\n` ending is part of the slice, and it must not reach a block's content
/// (it would land in the SVG `<text>` and in `text::measure`'s width). Only a
/// *trailing* CR is dropped — a lone CR mid-line is ordinary content, since the
/// grammar never sanctioned bare-CR line endings. The input is left on the
/// `\n`, which [`eol`] then consumes.
pub fn raw_line<'i>(i: &mut &'i str) -> ModalResult<&'i str> {
    let line: &'i str = take_till(0.., |c: char| c == '\n').parse_next(i)?;
    Ok(line.strip_suffix('\r').unwrap_or(line))
}

/// Double-quoted string with `\"`, `\\`, and `\n` escapes.
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
                'n' => out.push('\n'),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_escape_produces_a_newline() {
        let mut s = "\"Kubernetes\\nCluster\" rest";
        let out = string(&mut s).expect("parse");
        assert_eq!(out, "Kubernetes\nCluster");
        assert_eq!(s, " rest");
    }

    #[test]
    fn other_escapes_still_work() {
        let mut s = "\"a\\\"b\\\\c\"";
        let out = string(&mut s).expect("parse");
        assert_eq!(out, "a\"b\\c");
    }

    #[test]
    fn unknown_escape_is_an_error() {
        let mut s = "\"a\\tb\"";
        assert!(string(&mut s).is_err(), "\\t stays an error for v0");
    }

    // ---- raw body lines (M21.1) ----

    #[test]
    fn raw_line_takes_everything_verbatim() {
        // No comment stripping, no tokenizing: prose is prose.
        let mut s = "  # not a comment --> \"unterminated\nnext";
        assert_eq!(
            raw_line(&mut s).expect("parse"),
            "  # not a comment --> \"unterminated"
        );
        // The newline is left for `eol`.
        assert_eq!(s, "\nnext");
    }

    #[test]
    fn raw_line_drops_the_cr_of_a_crlf_ending() {
        // The CR of a `\r\n` ending must not reach a block's content, and the
        // `\n` is still there for `eol` to consume.
        let mut s = "text\r\nmore";
        assert_eq!(raw_line(&mut s).expect("parse"), "text");
        assert_eq!(s, "\nmore");
        eol(&mut s).expect("eol");
        assert_eq!(s, "more");
    }

    #[test]
    fn raw_line_keeps_an_interior_cr() {
        // Only a trailing CR is a line-ending artifact; a lone CR mid-line is
        // content (the grammar never sanctioned bare-CR line endings).
        let mut s = "a\rb\n";
        assert_eq!(raw_line(&mut s).expect("parse"), "a\rb");
    }

    #[test]
    fn raw_line_at_eof_is_the_last_line() {
        let mut s = "no trailing newline";
        assert_eq!(raw_line(&mut s).expect("parse"), "no trailing newline");
        assert_eq!(s, "");
    }

    #[test]
    fn raw_line_keeps_interior_and_trailing_spaces() {
        let mut s = "  a  b   \n";
        assert_eq!(raw_line(&mut s).expect("parse"), "  a  b   ");
    }
}
