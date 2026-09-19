//! diagrammer — a small DSL for infrastructure diagrams.
//!
//! v0 status: the input language is parsed and semantically validated end to
//! end, laid out (Sugiyama-style, including compound subgraphs with
//! per-subgraph direction), and rendered to a self-contained SVG. See
//! [`docs/grammar.md`](../docs/grammar.md) and the milestone plan in
//! [`docs/milestones.md`](../docs/milestones.md).
//!
//! Cross-boundary edge routing through subgraph frames (M5) and
//! shape/style/color rendering (M6) are parsed but not yet reflected in the
//! output; the resolved-model fields for them are carried through until those
//! milestones land.
#![allow(dead_code)]

mod ast;
mod error;
mod layout;
mod lexer;
mod parser;
mod render;
mod resolve;
mod text;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let prog = args.first().map(String::as_str).unwrap_or("diagrammer");

    if args.len() == 1 || args.iter().any(|a| a == "-h" || a == "--help") {
        let code = if args.len() == 1 { ExitCode::from(2) } else { ExitCode::SUCCESS };
        print!("{}", usage(prog));
        return code;
    }

    let cli = match parse_args(&args) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!();
            eprint!("{}", usage(prog));
            return ExitCode::from(2);
        }
    };

    let source = match std::fs::read_to_string(&cli.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", cli.input.display());
            return ExitCode::from(1);
        }
    };

    match run(&source, cli.output.as_deref()) {
        Ok(RunOutcome::Summary(s)) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Ok(RunOutcome::Wrote(path)) => {
            eprintln!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(error::Error::Parse { offset, message } | error::Error::Resolve { offset, message }) => {
            eprintln!("error at line {}: {message}", error::line_of(&source, offset));
            ExitCode::from(1)
        }
        Err(error::Error::Io(e)) => {
            eprintln!("io error: {e}");
            ExitCode::from(1)
        }
    }
}

/// What `run` produced: either a textual summary (no `-o`) or a written file.
enum RunOutcome {
    Summary(String),
    Wrote(PathBuf),
}

/// Parse and validate `source`; if `output` is given, lay it out, render SVG,
/// and write the file. Otherwise return a one-line summary.
fn run(source: &str, output: Option<&Path>) -> Result<RunOutcome, error::Error> {
    let raw = parser::parse_diagram(source)?;
    let diagram = resolve::resolve(&raw)?;
    match output {
        None => Ok(RunOutcome::Summary(format!(
            "ok: {} nodes, {} edges, {} subgraphs (direction {})",
            diagram.nodes.len(),
            diagram.edges.len(),
            diagram.subgraphs.len(),
            diagram.direction.as_str(),
        ))),
        Some(path) => {
            let layout = layout::layout(&diagram);
            let svg = render::svg::render_svg(&diagram, &layout);
            std::fs::write(path, svg)?;
            Ok(RunOutcome::Wrote(path.to_path_buf()))
        }
    }
}

/// Parsed command-line arguments.
struct Cli {
    input: PathBuf,
    output: Option<PathBuf>,
}

/// A tiny hand-rolled parser — the CLI has two options, so a dependency
/// would be more weight than it's worth.
fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-o" | "--output" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    return Err(format!("{a} requires a value"));
                };
                if output.is_some() {
                    return Err(format!("multiple output files given"));
                }
                output = Some(PathBuf::from(val));
            }
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option `{s}`"));
            }
            _ => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument `{a}`"));
                }
                input = Some(PathBuf::from(a));
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        return Err("missing input file".to_string());
    };
    Ok(Cli { input, output })
}

fn usage(prog: &str) -> String {
    format!(
        "usage: {prog} <input.mmd> [-o <output.svg>]\n\
         \n\
         Parses and validates a diagram. Without `-o` it prints a summary to\n\
         stdout; with `-o <output.svg>` it lays out the diagram and writes a\n\
         self-contained, GitHub-renderable SVG file.\n"
    )
}
