//! diagrammer — a small DSL for infrastructure diagrams.
//!
//! v0 status: the input language is parsed and semantically validated end to
//! end, laid out (Sugiyama-style, including compound groups with
//! per-group direction), and rendered to a self-contained SVG. Rendering
//! (M6) honors the `cylinder` shape, per-node `color`/`fill`, the
//! `dotted`/`dashed`/`thick` edge styles, per-edge `color`, and edge labels;
//! M7.5 adds per-group `color`/`fill`/`line` (border style) and a `text`
//! text-color attribute on nodes, edges, and groups.
//! See [`docs/grammar.md`](../docs/grammar.md) and the milestone plan in
//! [`docs/milestones.md`](../docs/milestones.md).

mod ast;
mod decorate;
mod error;
mod layout;
mod lexer;
mod metrics_table;
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
        let code = if args.len() == 1 {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
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

    let raw_source = match std::fs::read_to_string(&cli.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", cli.input.display());
            return ExitCode::from(1);
        }
    };
    let source = normalize_source(&raw_source);

    match run(&source, cli.output.as_deref()) {
        Ok(RunOutcome::Summary(s)) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Ok(RunOutcome::Wrote(path)) => {
            eprintln!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(
            error::Error::Parse { offset, message } | error::Error::Resolve { offset, message },
        ) => {
            eprintln!(
                "error at line {}: {message}",
                error::line_of(&source, offset)
            );
            ExitCode::from(1)
        }
        Err(error::Error::Io(e)) => {
            eprintln!("io error: {e}");
            ExitCode::from(1)
        }
    }
}

/// Normalize text-file artifacts a Windows editor or shell leaves behind, so a
/// `.dgmr` written anywhere parses identically.
///
/// Two things get fixed, both cheap and both common enough to matter on the
/// platform where they're produced:
///
/// - **A UTF-8 BOM.** Notepad and PowerShell 5.1 (`>` / `Out-File`) emit one.
///   The lexer has no rule for `U+FEFF`, so without this a BOM'd file fails
///   with the unhelpful `error at line 1: diagram header` — the single most
///   likely first experience for a Windows user. (UTF-16 files are a different
///   problem: `read_to_string` rejects them as invalid UTF-8, which is at least
///   an honest error.)
/// - **CRLF → LF.** `\r\n` already parses (the lexer's `eol` accepts it), so
///   this is not a correctness fix for ordinary input; it's for the label
///   escapes. A quoted `"a\r\nb"` keeps its literal `\r`, which then lands in
///   the SVG `<text>` and in `text::measure`'s width, where a bare CR is not
///   zero-width and is not normalized the way an XML parser normalizes it.
///
/// Normalizing here — at the one place bytes become a `str` — keeps the lexer
/// honest about its own grammar while making line endings a non-issue for
/// error offsets, too (`line_of` counts `\n`).
fn normalize_source(s: &str) -> String {
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    if s.contains('\r') {
        s.replace("\r\n", "\n")
    } else {
        s.to_string()
    }
}

/// What `run` produced: either a textual summary (no `-o`) or a written file.
#[cfg_attr(test, derive(Debug))]
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
            "ok: {} nodes, {} edges, {} groups (direction {}){}",
            diagram.nodes.len(),
            diagram.edges.len(),
            diagram.groups.len(),
            diagram.direction.as_str(),
            page_furniture_summary(&diagram),
        ))),
        Some(path) => {
            let layout = layout::layout(&diagram);
            let svg = render::svg::render_svg(&diagram, &layout);
            std::fs::write(path, svg)?;
            Ok(RunOutcome::Wrote(path.to_path_buf()))
        }
    }
}

/// A short page-furniture suffix for the CLI summary, empty when the diagram
/// uses none of it — so every diagram written before M21 keeps its summary
/// byte-for-byte.
///
/// The summary is a validation report, so it reports what `resolve` accepted:
/// each block, its fields/body size, and the style values the author actually
/// set. That also makes it the only production reader of the resolved
/// decoration until M21.3's `decorate` pass draws it — which matters because
/// the crate denies dead code rather than letting scaffolding accumulate.
fn page_furniture_summary(diagram: &ast::Diagram) -> String {
    /// `name`, then ` key=value` for each style that was set.
    fn with_styles(name: &str, styles: &[(&str, Option<String>)]) -> String {
        let mut out = name.to_string();
        for (key, value) in styles {
            if let Some(v) = value {
                out.push_str(&format!(" {key}={v}"));
            }
        }
        out
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(b) = &diagram.border {
        let line = b.line.map(|l| l.as_str().to_string());
        parts.push(with_styles(
            "border",
            &[("line", line), ("color", b.color.clone())],
        ));
    }
    if let Some(tb) = &diagram.title_block {
        // Fields read as `Key="value"` pairs — the table's content, which is
        // what a title block is for.
        let fields: Vec<String> = tb
            .fields
            .iter()
            .map(|f| format!("{}={:?}", f.key, f.value))
            .collect();
        parts.push(with_styles(
            &format!("titleblock [{}]", fields.join(", ")),
            &[("color", tb.color.clone()), ("fill", tb.fill.clone())],
        ));
    }
    for block in &diagram.text_blocks {
        let width = block.width.map(|w| w.to_string());
        parts.push(with_styles(
            &format!(
                "textblock {} ({} lines)",
                block.anchor.as_str(),
                block.body.len()
            ),
            &[
                ("width", width),
                ("color", block.color.clone()),
                ("fill", block.fill.clone()),
                ("text", block.text.clone()),
            ],
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("; page: {}", parts.join(", "))
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
                    return Err("multiple output files given".to_string());
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
        "usage: {prog} <input.dgmr> [-o <output.svg>]\n\
         \n\
         Parses and validates a diagram. Without `-o` it prints a summary to\n\
         stdout; with `-o <output.svg>` it lays out the diagram and writes a\n\
         self-contained, GitHub-renderable SVG file.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Source normalization (BOM / CRLF) ----

    #[test]
    fn utf8_bom_is_stripped() {
        assert_eq!(
            normalize_source("\u{feff}diagram top-down\n"),
            "diagram top-down\n"
        );
    }

    #[test]
    fn crlf_becomes_lf() {
        assert_eq!(normalize_source("a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn bom_and_crlf_together() {
        assert_eq!(normalize_source("\u{feff}a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn lone_cr_is_left_alone() {
        // Classic-Mac line endings are not a Windows artifact and the lexer
        // does not accept them; normalizing them here would silently invent
        // line breaks the grammar never sanctioned.
        assert_eq!(normalize_source("a\rb"), "a\rb");
    }

    #[test]
    fn normalization_is_idempotent() {
        for src in ["", "a\n", "a\r\n", "\u{feff}a\r\nb"] {
            let once = normalize_source(src);
            assert_eq!(normalize_source(&once), once, "not idempotent for {src:?}");
        }
    }

    #[test]
    fn only_a_leading_bom_is_stripped() {
        // A U+FEFF *inside* the text is a (weird) label character, not a BOM;
        // stripping it would corrupt the label rather than fix the file.
        assert_eq!(normalize_source("a\u{feff}b"), "a\u{feff}b");
    }

    // ---- End-to-end: the artifacts that actually broke parsing ----

    /// The canonical sample, in the four on-disk spellings a Windows user can
    /// plausibly produce, must all parse and resolve to the same diagram.
    #[test]
    fn windows_file_encodings_all_parse() {
        let lf = include_str!("../examples/infra.dgmr");
        let crlf = lf.replace('\n', "\r\n");
        let bom = format!("\u{feff}{lf}");
        let bom_crlf = format!("\u{feff}{crlf}");
        let expected =
            summary(run(&normalize_source(lf), None).expect("the LF baseline must parse"));
        for (name, variant) in [("crlf", crlf), ("bom", bom), ("bom+crlf", bom_crlf)] {
            let got = summary(
                run(&normalize_source(&variant), None)
                    .unwrap_or_else(|e| panic!("{name} spelling failed to parse: {e:?}")),
            );
            assert_eq!(got, expected, "{name} spelling resolved differently");
        }
    }

    /// A `\r\n` inside a quoted label used to leak a literal CR into the SVG
    /// text (and into `text::measure`'s width). After normalization the label
    /// is a clean two-line `<tspan>`.
    #[test]
    fn crlf_inside_a_label_does_not_leak_a_cr_into_the_svg() {
        let src = normalize_source("diagram top-down\r\n\r\na \"L1\r\nL2\" --> b\r\n");
        let svg = render(&src);
        assert!(!svg.contains('\r'), "a CR survived into the SVG: {svg:?}");
        assert!(
            svg.contains(">L1<tspan") && svg.contains(">L2</tspan>"),
            "the label did not become a clean two-line tspan: {svg}"
        );
    }

    // ---- M21.1: page furniture in the summary ----

    /// The invariant that keeps M21.1 output-neutral: a diagram using none of
    /// the page furniture keeps its summary byte-for-byte.
    #[test]
    fn summary_is_unchanged_when_no_page_furniture_is_used() {
        let out = summary(run(include_str!("../examples/infra.dgmr"), None).expect("parse"));
        assert_eq!(out, "ok: 8 nodes, 10 edges, 1 groups (direction top-down)");
        assert!(!out.contains("page:"), "no decoration, no suffix: {out}");
    }

    #[test]
    fn summary_reports_page_furniture_when_present() {
        let out = summary(
            run(
                "diagram top-down\n\
                 border line=\"dashed\"\n\
                 titleblock\nTitle = \"X\"\nCompany = \"Y\"\ncolor = \"#888\"\nend\n\
                 textblock right width=\"320\"\nhello\nworld\nend\n",
                None,
            )
            .expect("parse"),
        );
        assert!(
            out.starts_with("ok: 0 nodes, 0 edges, 0 groups (direction top-down)"),
            "{out}"
        );
        assert!(out.contains("page:"), "{out}");
        assert!(out.contains("border line=dashed"), "{out}");
        // `color` is a style key, so it is not one of the reported fields.
        assert!(
            out.contains("titleblock [Title=\"X\", Company=\"Y\"] color=#888"),
            "{out}"
        );
        assert!(out.contains("textblock right (2 lines) width=320"), "{out}");
    }

    // ---- M21.3: page furniture end-to-end ----

    /// A diagram using no page furniture renders exactly as it did before
    /// M21.3 existed. The snapshots pin the bytes; this pins the property that
    /// makes them safe: the furniture is invisible unless declared.
    #[test]
    fn page_furniture_is_absent_unless_declared() {
        let svg = render(include_str!("../examples/infra.dgmr"));
        for needle in ["#fbfbfb", "#fdf6ec"] {
            assert!(!svg.contains(needle), "unexpected page furniture: {needle}");
        }
    }

    /// A block statement inside a `group` body is a parse error with the right
    /// line number (the group-body trap M21.1 closed).
    #[test]
    fn block_in_a_group_body_is_a_parse_error_with_its_line() {
        let src = "diagram top-down\ngroup \"G\"\na\ntextblock right\nend\n";
        let err = run(src, None).expect_err("a block in a group body must fail");
        let (offset, message) = match err {
            crate::error::Error::Parse { offset, message }
            | crate::error::Error::Resolve { offset, message } => (offset, message),
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(
            message.contains("`textblock` cannot be a group member"),
            "{message}"
        );
        assert_eq!(crate::error::line_of(src, offset), 4);
    }

    /// A duplicate text-block anchor is a resolve error pointing at the second
    /// block.
    #[test]
    fn duplicate_text_block_anchor_is_a_resolve_error_with_its_line() {
        let src = "diagram top-down\ntextblock right\none\nend\ntextblock right\ntwo\nend\n";
        let err = run(src, None).expect_err("duplicate anchor must fail");
        let (offset, message) = match err {
            crate::error::Error::Parse { offset, message }
            | crate::error::Error::Resolve { offset, message } => (offset, message),
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(
            message.contains("duplicate text block anchor `right`"),
            "{message}"
        );
        assert_eq!(crate::error::line_of(src, offset), 5);
    }

    /// A `textblock lower-right` alongside a `titleblock` is a resolve error
    /// pointing at the text block (the statement that has to move), whatever
    /// order the two were written in.
    #[test]
    fn lower_right_text_block_conflicts_with_a_title_block() {
        let src = "diagram top-down\ntextblock lower-right\nx\nend\ntitleblock Title=\"T\"\nend\n";
        let err = run(src, None).expect_err("lower-right + title block must fail");
        let (offset, message) = match err {
            crate::error::Error::Parse { offset, message }
            | crate::error::Error::Resolve { offset, message } => (offset, message),
            other => panic!("unexpected error: {other:?}"),
        };
        assert!(message.contains("lower-right"), "{message}");
        assert_eq!(
            crate::error::line_of(src, offset),
            2,
            "the error points at the text block, not the title block: {message}"
        );
    }

    /// A page-furniture diagram renders end to end through the CLI path.
    #[test]
    fn page_furniture_renders_end_to_end() {
        let svg = render(
            "diagram top-down\n\
             border\n\
             titleblock Title=\"T\"\nend\n\
             textblock top\n hello\n end\n\
             a --> b\n",
        );
        assert!(svg.contains(">hello</text>"), "{svg}");
        assert!(svg.contains(">Title</text>"), "{svg}");
        // Content is still there, under the decoration.
        assert!(
            svg.contains(">a</text>") && svg.contains(">b</text>"),
            "{svg}"
        );
    }

    /// Render `src` the way `run` would with `-o`, without touching the disk.
    fn render(src: &str) -> String {
        let raw = crate::parser::parse_diagram(src).expect("parse");
        let diagram = crate::resolve::resolve(&raw).expect("resolve");
        let layout = crate::layout::layout(&diagram);
        crate::render::svg::render_svg(&diagram, &layout)
    }

    fn summary(outcome: RunOutcome) -> String {
        match outcome {
            RunOutcome::Summary(s) => s,
            RunOutcome::Wrote(p) => format!("wrote {}", p.display()),
        }
    }
}
