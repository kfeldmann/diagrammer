//! Regenerates `src/metrics_table.rs` from the bundled DejaVu Sans **Regular**
//! and **Bold** faces.
//!
//! Run from the workspace root with:
//!
//! ```text
//! cargo run -p gen-metrics-table
//! ```
//!
//! The generated tables bake exactly the font lookups `text::measure_run` needs
//! (char → glyph id, glyph advance, kerning, ascent/descent), replicating the
//! lookup semantics of `ttf-parser` so measurement stays byte-identical:
//!
//! - char → glyph: `Face::glyph_index` over every codepoint enumerated from
//!   the Unicode `cmap` subtables; unmapped chars fall back to `.notdef`
//!   (glyph 0), which the runtime lookup reproduces.
//! - advance: `Face::glyph_hor_advance(i).unwrap_or(0)` for every glyph id
//!   (replicating the last-resort `hmtx` behavior).
//! - kerning: first horizontal `kern` subtable containing the pair wins
//!   (the former `find_map` semantics); pairs whose effective value is 0 are
//!   omitted — a missing pair measures as 0 exactly like a zero value.
//!
//! **Both faces are emitted with identical semantics.** They are separate
//! tables because DejaVu Sans Bold is materially wider than Regular (whole-
//! string ratios of 1.12–1.23 on prose), so measuring a bold run with Regular
//! advances would overflow its box. DejaVu Sans *Oblique* needs no table: its
//! em box matches Regular's and, across ASCII and Latin-1, its advances do too
//! except for four glyphs (see `text::Weight`), so italic measurement reuses
//! the Regular table by design.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ttf_parser::kern::Format;
use ttf_parser::{Face, GlyphId};

/// One face's baked data, ready to emit.
struct Tables {
    /// Identifier of the emitted `const` (`REGULAR` / `BOLD`).
    ident: &'static str,
    /// Human name for the doc comment.
    name: &'static str,
    ascent: i16,
    descent: i16,
    /// `head.unitsPerEm` — the divisor CSS/SVG use to turn font units into
    /// pixels. **Not** the same as `ascent - descent` (2384 for DejaVu Sans,
    /// where `unitsPerEm` is 2048).
    units_per_em: u16,
    /// char → glyph as `(char_start, gid_start, len)` runs.
    runs: Vec<(u32, u16, u16)>,
    /// Advance per glyph id, in font units.
    advances: Vec<u16>,
    /// `(pair, value)` with `pair = left_glyph << 16 | right_glyph`.
    kern: Vec<(u32, i16)>,
}

fn extract(ident: &'static str, name: &'static str, data: &'static [u8]) -> Tables {
    let face = Face::parse(data, 0)
        .unwrap_or_else(|e| panic!("bundled DejaVu Sans {name} must parse: {e:?}"));

    // char -> glyph id. Authoritative `glyph_index` lookup per codepoint, so
    // subtable overlap/precedence is resolved exactly like the runtime did.
    let mut chars: BTreeMap<u32, u16> = BTreeMap::new();
    for sub in face
        .tables()
        .cmap
        .iter()
        .flat_map(|c| c.subtables)
        .filter(|s| s.is_unicode())
    {
        sub.codepoints(|cp| {
            let Some(c) = char::from_u32(cp) else { return };
            if let Some(gid) = face.glyph_index(c) {
                // gid 0 (`.notdef`) is the runtime fallback; no entry needed.
                if gid.0 != 0 {
                    chars.insert(cp, gid.0);
                }
            }
        });
    }

    // Runs of consecutive codepoints mapping to consecutive glyph ids:
    // char_start + i -> gid_start + i, for i in 0..len.
    let mut runs: Vec<(u32, u16, u16)> = Vec::new();
    let mut iter = chars.into_iter();
    if let Some((mut run_c, mut run_g)) = iter.next() {
        let (mut start_c, mut start_g, mut len) = (run_c, run_g, 1u32);
        for (c, g) in iter {
            if c == run_c + 1 && g == run_g.wrapping_add(1) {
                len += 1;
            } else {
                runs.push((start_c, start_g, len as u16));
                (start_c, start_g, len) = (c, g, 1);
            }
            (run_c, run_g) = (c, g);
        }
        runs.push((start_c, start_g, len as u16));
    }

    // Per-glyph advances in font units (0 where the font reports none).
    let advances: Vec<u16> = (0..face.number_of_glyphs())
        .map(|i| face.glyph_hor_advance(GlyphId(i)).unwrap_or(0))
        .collect();

    // Effective kerning per pair: first horizontal subtable with a value wins.
    let mut kern: BTreeMap<u32, i16> = BTreeMap::new();
    if let Some(table) = face.tables().kern {
        for st in table.subtables.into_iter().filter(|st| st.horizontal) {
            match st.format {
                Format::Format0(f0) => {
                    for pair in f0.pairs {
                        kern.entry(pair.pair).or_insert(pair.value);
                    }
                }
                other => {
                    panic!("unsupported kern subtable format: {other:?} (extend the generator)")
                }
            }
        }
    }
    kern.retain(|_, v| *v != 0); // zero == absent for measurement purposes

    Tables {
        ident,
        name,
        ascent: face.ascender(),
        descent: face.descender(),
        units_per_em: face.units_per_em(),
        runs,
        advances,
        kern: kern.into_iter().collect(),
    }
}

/// Emit one face as top-level data consts plus a `Face` that points at them.
///
/// The arrays are emitted at module scope (not nested inside the `Face`
/// literal) so `rustfmt` can wrap them — the generated file is formatted like
/// hand-written source and `cargo fmt --check` stays clean.
fn write_face(out: &mut String, t: &Tables) {
    let upper = t.ident;

    writeln!(out, "// ---- DejaVu Sans {} ----", t.name).unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// char → glyph id runs for DejaVu Sans {}: `(char_start, gid_start, len)`\n\
         /// — codepoint `char_start + i` maps to glyph `gid_start + i` for\n\
         /// `i in 0..len`. Sorted by `char_start` for binary search; codepoints\n\
         /// not covered map to glyph 0 (`.notdef`).\n\
         ///\n\
         /// ({} runs covering {} codepoints, {} glyphs in the font.)",
        t.name,
        t.runs.len(),
        t.runs.iter().map(|r| r.2 as usize).sum::<usize>(),
        t.advances.len(),
    )
    .unwrap();
    writeln!(out, "const {upper}_CHAR_RUNS: &[(u32, u16, u16)] = &[").unwrap();
    for (c, g, len) in &t.runs {
        writeln!(out, "    ({c}, {g}, {len}),").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// Horizontal advance in font units, indexed by glyph id, for DejaVu\n\
         /// Sans {}. ({} entries.)",
        t.name,
        t.advances.len()
    )
    .unwrap();
    writeln!(out, "const {upper}_ADVANCES: &[u16] = &[").unwrap();
    for chunk in t.advances.chunks(12) {
        let line: Vec<String> = chunk.iter().map(|a| a.to_string()).collect();
        writeln!(out, "    {},", line.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// Kerning in font units for DejaVu Sans {}, as `(pair, value)` where\n\
         /// `pair` is `(left_glyph << 16) | right_glyph`. Sorted by `pair` for\n\
         /// binary search; a missing pair kerns as 0. ({} pairs; zero-valued\n\
         /// pairs are omitted.)",
        t.name,
        t.kern.len()
    )
    .unwrap();
    writeln!(out, "const {upper}_KERN: &[(u32, i16)] = &[").unwrap();
    for (pair, value) in &t.kern {
        writeln!(out, "    ({pair}, {value}),").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// Baked DejaVu Sans {} metrics.", t.name).unwrap();
    writeln!(out, "pub const {upper}: Face = Face {{").unwrap();
    writeln!(out, "    ascent: {},", t.ascent).unwrap();
    writeln!(out, "    descent: {},", t.descent).unwrap();
    writeln!(out, "    units_per_em: {},", t.units_per_em).unwrap();
    writeln!(out, "    char_runs: {upper}_CHAR_RUNS,").unwrap();
    writeln!(out, "    advances: {upper}_ADVANCES,").unwrap();
    writeln!(out, "    kern: {upper}_KERN,").unwrap();
    writeln!(out, "}};").unwrap();
    writeln!(out).unwrap();
}

/// Run the workspace `rustfmt` over generated source, returning the formatted
/// text.
///
/// Falls back to the unformatted text (with a warning) if `rustfmt` is not
/// installed: the file is still correct, only its line wrapping differs, and
/// failing the regeneration over a formatting tool would be worse than a
/// noisy `cargo fmt --check` later.
fn rustfmt(source: String, path: &std::path::Path) -> String {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: could not run rustfmt ({e}); writing unformatted table");
            return source;
        }
    };
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(source.as_bytes())
        .expect("write to rustfmt");
    let output = child.wait_with_output().expect("rustfmt runs");
    if !output.status.success() {
        eprintln!(
            "warning: rustfmt failed on {} ({}); writing unformatted table",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return source;
    }
    String::from_utf8(output.stdout).expect("rustfmt emits UTF-8")
}

fn main() {
    let regular = extract("REGULAR", "Regular", dejavu::sans::regular());
    let bold = extract("BOLD", "Bold", dejavu::sans::bold());

    let mut out = String::new();
    writeln!(
        out,
        "//! Baked DejaVu Sans metrics (Regular and Bold) — GENERATED FILE, DO NOT EDIT."
    )
    .unwrap();
    writeln!(out, "//!").unwrap();
    writeln!(
        out,
        "//! Regenerate with `cargo run -p gen-metrics-table` (see the"
    )
    .unwrap();
    writeln!(
        out,
        "//! tool's header comment for the exact semantics baked here). Values"
    )
    .unwrap();
    writeln!(
        out,
        "//! are in font units (em = `unitsPerEm`); `text::measure_run`"
    )
    .unwrap();
    writeln!(
        out,
        "//! scales them to pixels. Both faces are baked with identical"
    )
    .unwrap();
    writeln!(
        out,
        "//! semantics, from the two fonts the `dejavu` crate bundles; the"
    )
    .unwrap();
    writeln!(
        out,
        "//! `font_verification` tests prove each table matches live"
    )
    .unwrap();
    writeln!(
        out,
        "//! `ttf-parser` lookups against its own font byte for byte."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// One face's baked metrics, in font units. `text::measure_run` scales"
    )
    .unwrap();
    writeln!(
        out,
        "/// these to pixels at the requested font size; the two faces differ in"
    )
    .unwrap();
    writeln!(
        out,
        "/// every field but the em box (they share `unitsPerEm`, so line"
    )
    .unwrap();
    writeln!(out, "/// heights are weight-independent).").unwrap();
    writeln!(out, "pub struct Face {{").unwrap();
    writeln!(out, "    /// `hhea` ascender.").unwrap();
    writeln!(out, "    pub ascent: i16,").unwrap();
    writeln!(out, "    /// `hhea` descender (negative).").unwrap();
    writeln!(out, "    pub descent: i16,").unwrap();
    writeln!(
        out,
        "    /// `head.unitsPerEm` — the divisor that turns font units into pixels."
    )
    .unwrap();
    writeln!(
        out,
        "    /// This is what a renderer uses, and it is **not** `ascent - descent`."
    )
    .unwrap();
    writeln!(out, "    pub units_per_em: u16,").unwrap();
    writeln!(
        out,
        "    /// char → glyph id runs (see the emitted comment)."
    )
    .unwrap();
    writeln!(out, "    pub char_runs: &'static [(u32, u16, u16)],").unwrap();
    writeln!(out, "    /// Advance per glyph id.").unwrap();
    writeln!(out, "    pub advances: &'static [u16],").unwrap();
    writeln!(out, "    /// Kerning pairs, sorted by `pair`.").unwrap();
    writeln!(out, "    pub kern: &'static [(u32, i16)],").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    write_face(&mut out, &regular);
    write_face(&mut out, &bold);

    // Emit the file already formatted. The advance arrays are long enough
    // that rustfmt reflows them, and a generated file that fails
    // `cargo fmt --check` makes every later `cargo fmt` produce a diff the
    // author has to reason about. Formatting here keeps "regenerate" a
    // single command with no follow-up step.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/metrics_table.rs");
    let out = rustfmt(out, &path);
    std::fs::write(&path, out).expect("write workspace src/metrics_table.rs");
    for t in [&regular, &bold] {
        eprintln!(
            "{}: {} runs, {} advances, {} kern pairs",
            t.ident,
            t.runs.len(),
            t.advances.len(),
            t.kern.len()
        );
    }
    eprintln!("wrote src/metrics_table.rs");
}
