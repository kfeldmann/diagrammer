//! Regenerates `src/metrics_table.rs` from the bundled DejaVu Sans Regular.
//!
//! Run from the workspace root with:
//!
//! ```text
//! cargo run -p gen-metrics-table
//! ```
//!
//! The generated table bakes exactly the font lookups `text::measure` needs
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

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ttf_parser::kern::Format;
use ttf_parser::{Face, GlyphId};

fn main() {
    let face = Face::parse(dejavu::sans::regular(), 0)
        .expect("bundled DejaVu Sans Regular must parse as a valid font");

    // char -> glyph id. Authoritative `glyph_index` lookup per codepoint, so
    // subtable overlap/precedence is resolved exactly like the runtime did.
    let mut chars: BTreeMap<u32, u16> = BTreeMap::new();
    for sub in face.tables().cmap.iter().flat_map(|c| c.subtables).filter(|s| s.is_unicode()) {
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
                other => panic!("unsupported kern subtable format: {other:?} (extend the generator)"),
            }
        }
    }
    kern.retain(|_, v| *v != 0); // zero == absent for measurement purposes

    let kern: Vec<(u32, i16)> = kern.into_iter().collect();

    let mut out = String::new();
    writeln!(out, "//! Baked DejaVu Sans Regular metrics — GENERATED FILE, DO NOT EDIT.").unwrap();
    writeln!(out, "//!").unwrap();
    writeln!(out, "//! Regenerate with `cargo run -p gen-metrics-table` (see the").unwrap();
    writeln!(out, "//! tool's header comment for the exact semantics baked here). Values").unwrap();
    writeln!(out, "//! are in font units (em = `ASCENT - DESCENT`); `text::measure` scales").unwrap();
    writeln!(out, "//! them to pixels. The `font_verification` tests prove this table").unwrap();
    writeln!(out, "//! matches live `ttf-parser` lookups against the bundled font byte for").unwrap();
    writeln!(out, "//! byte.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// `hhea` ascender, in font units.").unwrap();
    writeln!(out, "pub const ASCENT: i16 = {};", face.ascender()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// `hhea` descender (negative), in font units.").unwrap();
    writeln!(out, "pub const DESCENT: i16 = {};", face.descender()).unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// char → glyph id as `(char_start, gid_start, len)` runs: codepoint\n\
         /// `char_start + i` maps to glyph `gid_start + i` for `i in 0..len`.\n\
         /// Sorted by `char_start` for binary search; codepoints not covered map\n\
         /// to glyph 0 (`.notdef`).\n\
         ///\n\
         /// ({} runs covering {} codepoints, {} glyphs in the font.)",
        runs.len(),
        runs.iter().map(|r| r.2 as usize).sum::<usize>(),
        advances.len(),
    )
    .unwrap();
    writeln!(out, "pub const CHAR_RUNS: &[(u32, u16, u16)] = &[").unwrap();
    for (c, g, len) in &runs {
        writeln!(out, "    ({c}, {g}, {len}),").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// Horizontal advance in font units, indexed by glyph id.\n/// ({} entries.)",
        advances.len()
    )
    .unwrap();
    writeln!(out, "pub const ADVANCES: &[u16] = &[").unwrap();
    for chunk in advances.chunks(12) {
        let line: Vec<String> = chunk.iter().map(|a| a.to_string()).collect();
        writeln!(out, "    {},", line.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// Kerning in font units as `(pair, value)` where `pair` is\n\
         /// `(left_glyph << 16) | right_glyph`. Sorted by `pair` for binary\n\
         /// search; a missing pair kerns as 0. ({} pairs; zero-valued pairs\n\
         /// are omitted.)",
        kern.len()
    )
    .unwrap();
    writeln!(out, "pub const KERN: &[(u32, i16)] = &[").unwrap();
    for (pair, value) in &kern {
        writeln!(out, "    ({pair}, {value}),").unwrap();
    }
    writeln!(out, "];").unwrap();

    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/metrics_table.rs"),
        out,
    )
    .expect("write workspace src/metrics_table.rs");
    eprintln!(
        "wrote src/metrics_table.rs: {} runs, {} advances, {} kern pairs",
        runs.len(),
        advances.len(),
        kern.len()
    );
}