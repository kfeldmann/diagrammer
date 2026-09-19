//! Error types for diagrammer.
//!
//! All errors carry a byte `offset` into the source so the CLI can report a
//! 1-based line number. Parse errors come from the parser (syntax); resolve
//! errors come from semantic validation (e.g. conflicting redeclarations,
//! unknown attributes, nodes placed in more than one subgraph).

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse { offset: usize, message: String },
    Resolve { offset: usize, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Parse { message, .. } => write!(f, "{message}"),
            Error::Resolve { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Compute the 1-based line number of `offset` within `source`.
///
/// `offset` is a byte offset (as produced by the parser); it is floored to a
/// UTF-8 char boundary to avoid panicking on a mid-codepoint offset.
pub fn line_of(source: &str, offset: usize) -> usize {
    let mut upto = source.len().min(offset);
    while upto > 0 && !source.is_char_boundary(upto) {
        upto -= 1;
    }
    source[..upto].bytes().filter(|&b| b == b'\n').count() + 1
}
