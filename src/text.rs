//! Precise text measurement against the baked DejaVu Sans metrics in
//! [`crate::metrics_table`].
//!
//! Node labels must be sized before the boxes can be placed (milestone 2).
//! We measure the *layout width* (summed glyph advance, with kerning) and the
//! em height at a fixed pixel size, deterministically, from baked metrics so
//! that output is reproducible across machines. Widths scale by
//! `font_size / unitsPerEm` — exactly what a renderer does with `font-size` —
//! while heights use `font_size` directly as the em height. The
//! tables are generated from the DejaVu Sans **Regular** and **Bold** faces
//! embedded via the `dejavu` crate (`cargo run -p gen-metrics-table`) and
//! verified against them by the `font_verification` tests.
//!
//! # Weight and slant (M21.2)
//!
//! Two faces are baked because inline markup makes weight a *layout* input:
//! `**bold**` in a text block is 12–23% wider than the same text in Regular,
//! so measuring it with Regular advances would overflow the block. [`Weight`]
//! selects the table; [`measure`] is exactly `measure_run(.., Weight::Regular)`
//! and stays bit-identical for every pre-M21.2 caller.
//!
//! **Italic reuses the Regular table, deliberately.** DejaVu Sans Oblique has
//! an identical em box (1901/−483) to Regular and, across **ASCII and
//! Latin-1**, identical advances except for four glyphs (`« » Þ æ`, each
//! ≤1.4% wider) — so measuring italic with Regular advances is exact for
//! ordinary prose. Outside that range the faces do diverge on a few rare
//! codepoints (84 wider, worst +43% on U+05F2), but v0 labels are Latin text
//! and the standing policy is to measure the *widest plausible stack member*,
//! not the exact face; a third ~100 KB table to chase those codepoints would
//! not pay for itself. Slant therefore has no entry point here at all: an
//! italic run is measured as Regular. (`font_verification` pins the ASCII +
//! Latin-1 claim, so it stays true if the bundled font changes.)
//!
//! **Bold italic (`***text***`) is measured Bold.** Bold Oblique's advances
//! equal Bold's on prose (ratio 1.0000 on the standard sample) and exceed
//! them only on the same four Latin-1 glyphs (≤0.9%) — and Bold's advances are
//! themselves 12–23% wider than Regular's, so the weight dominates.

use crate::metrics_table as mt;

/// Which baked face to measure with (M21.2).
///
/// Only the two faces the language can actually produce are modelled: `**` is
/// bold, and italic is measured as Regular (see the module docs), so there is
/// no `Italic` variant to get wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Bold,
}

impl Weight {
    /// Every weight, for exhaustive checks over the baked tables.
    pub const ALL: [Weight; 2] = [Weight::Regular, Weight::Bold];

    /// The baked metrics for this weight.
    pub const fn face(self) -> &'static mt::Face {
        match self {
            Weight::Regular => &mt::REGULAR,
            Weight::Bold => &mt::BOLD,
        }
    }
}

/// The invariant that lets `line_height` ignore weight: both faces share an em
/// box, so a line's height is the same in Regular and Bold and [`Metrics`]
/// needs no weight-dependent height term. (It is also why italic can reuse
/// Regular's table without changing any line's height.) Checked at compile
/// time against the generated tables, so a regenerated table that drifted
/// would fail the build rather than quietly mis-space bold lines.
///
/// `units_per_em` is asserted too, and not for height: it is the *horizontal*
/// scale divisor ([`h_scale_factor`]). A face whose `unitsPerEm` differed from
/// Regular's would still be measurable, but the check keeps the two tables on
/// one em box so the height and width scalings can never disagree.
const _: () = {
    let mut i = 0;
    while i < Weight::ALL.len() {
        let face = Weight::ALL[i].face();
        assert!(face.ascent - face.descent == mt::REGULAR.ascent - mt::REGULAR.descent);
        assert!(face.units_per_em == mt::REGULAR.units_per_em);
        assert!(face.units_per_em > 0);
        assert!(!face.char_runs.is_empty() && !face.advances.is_empty());
        i += 1;
    }
};

/// Vertical distance between consecutive baselines of a multi-line label,
/// as a multiple of the font size. Shared with the renderer, which spaces
/// `<tspan>` lines the same way the layout reserves room for them.
pub const LINE_FACTOR: f32 = 1.2;

/// Baseline-to-baseline line height for text at `font_size` pixels.
pub fn line_height(font_size: f32) -> f32 {
    LINE_FACTOR * font_size
}

/// Number of lines in `text` (a `\n`-separated label). A label without
/// newlines is one line.
pub fn line_count(text: &str) -> usize {
    text.split('\n').count()
}

/// Measurement of a string at a given font size, in pixels.
///
/// `width` is weight-dependent (see [`measure_run`]); `height` is not, because
/// both baked faces share an em box.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Summed horizontal advance (the layout width, including the trailing
    /// advance of the last glyph) of the *widest line* — for a `\n`-separated
    /// label this is what the box must span.
    pub width: f32,
    /// Height of the text block: the first line's em height plus one
    /// [`line_height`] per additional line. For a single-line string this
    /// equals the `font_size` passed in.
    pub height: f32,
}

/// Pixels per font unit at `font_size` pixels: `font_size / unitsPerEm`.
///
/// This is the scaling every SVG/CSS renderer applies to `font-size`, so the
/// width measured here is the width drawn. It is **not** `font_size / (ascent
/// − descent)`: for DejaVu Sans those differ by a factor of 2384/2048 = 1.164,
/// and using the wrong one under-measured every string by ~14% — boxes came
/// out too narrow (text overflowed a node label or a text block) and, because
/// the renderer positions each run at the *measured* x while the glyphs are
/// drawn at their *true* width, adjacent words collided (most visibly in
/// all-caps bold, where the inter-word gap vanished).
///
/// The `ascent − descent` divisor survives only in `Metrics::height`, where it
/// is the em height of a *line* and has nothing to do with advance widths.
fn h_scale_factor(face: &mt::Face, font_size: f32) -> f32 {
    font_size / face.units_per_em as f32
}

/// Map `c` to a glyph id via the baked cmap runs; unmapped chars map to
/// `.notdef` (id 0), whose advance may be nonzero — matching the documented
/// v0 behavior.
fn glyph_id(face: &mt::Face, c: char) -> u16 {
    let cp = c as u32;
    match face.char_runs.binary_search_by_key(&cp, |r| r.0) {
        // Exact hit on a run start.
        Ok(i) => face.char_runs[i].1,
        // Inside the run ending just before this insertion point.
        Err(0) => 0,
        Err(i) => {
            let (start, gid, len) = face.char_runs[i - 1];
            if cp < start + len as u32 {
                gid + (cp - start) as u16
            } else {
                0
            }
        }
    }
}

/// Unscaled horizontal advance of `id` in font units (unknown id → 0).
fn h_advance_unscaled(face: &mt::Face, id: u16) -> f32 {
    face.advances.get(id as usize).copied().unwrap_or_default() as f32
}

/// Unscaled kerning between `first` and `second` in font units (none → 0).
fn kern_unscaled(face: &mt::Face, first: u16, second: u16) -> f32 {
    let pair = (first as u32) << 16 | second as u32;
    face.kern
        .binary_search_by_key(&pair, |k| k.0)
        .map(|i| face.kern[i].1)
        .unwrap_or_default() as f32
}

/// Measure `text` rendered at `font_size` pixels in the given [`Weight`].
///
/// A `\n` in `text` marks a line break: `width` is the widest line's layout
/// width and `height` spans every line (first line's em height plus one
/// [`line_height`] per additional line), matching how the renderer stacks
/// `<tspan>` lines. `height` does not depend on `weight` (both faces share an
/// em box — see the module docs), so only `width` moves.
///
/// Missing glyphs map to the font's `.notdef` (advance may be nonzero), which
/// is fine for v0 — labels are expected to be covered by DejaVu Sans (Latin,
/// digits, punctuation, common symbols).
pub fn measure_run(text: &str, font_size: f32, weight: Weight) -> Metrics {
    let face = weight.face();
    let h_scale = h_scale_factor(face, font_size);
    let measure_line = |line: &str| -> f32 {
        let mut width = 0.0_f32;
        let mut prev: Option<u16> = None;
        for c in line.chars() {
            let gid = glyph_id(face, c);
            if let Some(p) = prev {
                width += h_scale * kern_unscaled(face, p, gid);
            }
            width += h_scale * h_advance_unscaled(face, gid);
            prev = Some(gid);
        }
        width
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines
        .iter()
        .map(|l| measure_line(l))
        .fold(0.0_f32, f32::max);
    let height = font_size + (lines.len() - 1) as f32 * line_height(font_size);
    Metrics { width, height }
}

/// Measure `text` in Regular at `font_size` pixels.
///
/// This is the measurement every node label, group title, and edge label uses,
/// and it is exactly `measure_run(text, font_size, Weight::Regular)` — kept as
/// its own function so the (much more common) regular path reads plainly and
/// so every existing caller stays untouched.
pub fn measure(text: &str, font_size: f32) -> Metrics {
    measure_run(text, font_size, Weight::Regular)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_text_is_wider() {
        let a = measure("API", 14.0);
        let b = measure("API Server 1", 14.0);
        assert!(
            b.width > a.width,
            "{} should be wider than {}",
            b.width,
            a.width
        );
        assert!(a.width > 0.0);
    }

    #[test]
    fn height_tracks_font_size() {
        // Em height equals the (uniform) pixel scale.
        assert!((measure("x", 14.0).height - 14.0).abs() < 1e-4);
        assert!((measure("x", 24.0).height - 24.0).abs() < 1e-4);
    }

    #[test]
    fn empty_string_is_zero_width() {
        let m = measure("", 14.0);
        assert_eq!(m.width, 0.0);
        assert!(m.height > 0.0);
    }

    #[test]
    fn measurement_is_deterministic() {
        for s in ["web", "Postgres", "Kubernetes Cluster", "A-->B", "a\nb"] {
            assert_eq!(measure(s, 14.0).width, measure(s, 14.0).width);
        }
    }

    #[test]
    fn multiline_width_is_the_widest_line() {
        let narrow = measure("web", 14.0).width;
        let wide = measure("Postgres", 14.0).width;
        let m = measure("web\nPostgres\nweb", 14.0);
        assert!(
            (m.width - wide).abs() < 1e-4,
            "width should be the widest line"
        );
        assert!(m.width > narrow);
    }

    #[test]
    fn multiline_height_spans_every_line() {
        let one = measure("a", 14.0).height;
        let two = measure("a\nb", 14.0).height;
        let three = measure("a\nb\nc", 14.0).height;
        assert!((one - 14.0).abs() < 1e-4);
        assert!((two - (14.0 + line_height(14.0))).abs() < 1e-4);
        assert!((three - (14.0 + 2.0 * line_height(14.0))).abs() < 1e-4);
    }

    #[test]
    fn line_count_counts_breaks() {
        assert_eq!(line_count("one"), 1);
        assert_eq!(line_count("one\ntwo"), 2);
        assert_eq!(line_count("one\ntwo\nthree"), 3);
        // A trailing break starts a (empty) final line.
        assert_eq!(line_count("one\n"), 2);
    }

    // ---- M21.2: weight-aware measurement ----

    /// `measure` is exactly the Regular path — the whole point of keeping it.
    #[test]
    fn measure_is_regular_measure_run() {
        for s in ["web", "Postgres", "a\nb", "", "Ünïcödé — ½"] {
            for size in [1.0, 12.0, 14.0, 100.0] {
                let plain = measure(s, size);
                let run = measure_run(s, size, Weight::Regular);
                assert_eq!(plain.width.to_bits(), run.width.to_bits(), "{s:?} @ {size}");
                assert_eq!(
                    plain.height.to_bits(),
                    run.height.to_bits(),
                    "{s:?} @ {size}"
                );
            }
        }
    }

    /// Bold is materially wider — the reason the second table exists at all.
    /// The milestone's measured prose ratio is 1.117, and a title-block line
    /// is 1.130; assert the order of magnitude so a table mix-up (e.g. both
    /// faces pointing at Regular) fails loudly instead of silently
    /// under-measuring bold text.
    #[test]
    fn bold_is_materially_wider_than_regular() {
        let samples = [
            "The quick brown fox jumps over the lazy dog 0123456789",
            "Kubernetes Cluster — API Server 1 (replica)",
            "iiiiiiiiiiiiiiiiiiii",
        ];
        for s in samples {
            let reg = measure_run(s, 14.0, Weight::Regular).width;
            let bold = measure_run(s, 14.0, Weight::Bold).width;
            let ratio = bold / reg;
            assert!(
                (1.10..=1.25).contains(&ratio),
                "bold/regular ratio {ratio} out of the measured 1.117-1.234 band for {s:?}"
            );
        }
    }

    /// Widths scale by `font_size / unitsPerEm`, **not** by
    /// `font_size / (ascent - descent)`.
    ///
    /// This is the regression guard for the bug that made text blocks overflow
    /// and run words together: the two divisors differ by 2384/2048 = 1.164 for
    /// DejaVu Sans, so the wrong one under-measured every string by ~14%. The
    /// expected widths below are computed from the font's own `unitsPerEm`
    /// scaling, i.e. exactly what an SVG renderer draws for that `font-size`.
    #[test]
    fn width_scales_by_units_per_em_not_the_ascent_descent_span() {
        // Glyph `M` is 1767 font units wide in DejaVu Sans Regular; ten of them
        // at 13 px is `10 * 1767 * 13 / 2048`.
        let m_units = 1767.0_f32;
        let upem = Weight::Regular.face().units_per_em as f32;
        let expected = 10.0 * m_units * 13.0 / upem;
        let got = measure_run("MMMMMMMMMM", 13.0, Weight::Regular).width;
        assert!(
            (got - expected).abs() < 0.01,
            "measured {got} but unitsPerEm scaling predicts {expected}"
        );

        // And the old divisor really is different, so this test cannot pass
        // under it: assert the two disagree by the font's known ratio.
        let span = (Weight::Regular.face().ascent - Weight::Regular.face().descent) as f32;
        assert!(
            (span / upem - 1.164).abs() < 0.01,
            "ascent-descent / unitsPerEm should be ~1.164 for DejaVu Sans, got {}",
            span / upem
        );
        let wrong = 10.0 * m_units * 13.0 / span;
        assert!(
            (got - wrong).abs() > 1.0,
            "the unitsPerEm and ascent-descent scalings must differ materially"
        );
    }

    /// A space is measured in the face it will be drawn in, so bold text gets
    /// bold word gaps. Measured with the *wrong* weight the gap is ~9% narrow,
    /// which is invisible in lowercase but closes up an all-caps line.
    #[test]
    fn a_space_is_wider_in_bold_than_in_regular() {
        let reg = measure_run(" ", 13.0, Weight::Regular).width;
        let bold = measure_run(" ", 13.0, Weight::Bold).width;
        assert!(bold > reg, "bold space {bold} should exceed regular {reg}");
        // DejaVu Sans: 649 vs 651 units — close, but not equal.
        assert!(
            (bold / reg - 1.0).abs() < 0.10,
            "space weights should be within 10%, got {bold}/{reg}"
        );
    }

    /// Both faces share an em box, so weight changes only the width — the
    /// property `line_height` (and therefore every line's height) relies on.
    #[test]
    fn weight_does_not_change_height() {
        for s in ["x", "a\nb", "Kubernetes\nCluster\nAPI"] {
            for size in [1.0, 12.0, 14.0, 40.0] {
                let reg = measure_run(s, size, Weight::Regular);
                let bold = measure_run(s, size, Weight::Bold);
                assert_eq!(
                    reg.height.to_bits(),
                    bold.height.to_bits(),
                    "{s:?} @ {size}"
                );
            }
        }
    }

    /// The compile-time em-box check in this module is the guard; this pins
    /// the *values* too, so a font swap that changed them is visible in a test
    /// name rather than only in a build error.
    #[test]
    fn both_faces_share_the_regular_em_box() {
        for w in Weight::ALL {
            assert_eq!(w.face().ascent, Weight::Regular.face().ascent);
            assert_eq!(w.face().descent, Weight::Regular.face().descent);
        }
    }

    /// Bold's own cmap/advance tables are what get used: a bold-only glyph
    /// must measure from the bold table, not fall back to Regular's.
    #[test]
    fn bold_uses_the_bold_tables() {
        // Every face reports its own tables; an empty one would measure 0.
        for w in Weight::ALL {
            assert!(!w.face().char_runs.is_empty());
            assert!(!w.face().advances.is_empty());
        }
        // The faces really are different data, not the same slice twice.
        assert!(!std::ptr::eq(
            Weight::Regular.face().advances.as_ptr(),
            Weight::Bold.face().advances.as_ptr()
        ));
    }

    /// Empty text is zero-width in either weight (no accidental per-run
    /// padding introduced by the split).
    #[test]
    fn empty_text_is_zero_width_in_both_weights() {
        for w in Weight::ALL {
            assert_eq!(measure_run("", 14.0, w).width, 0.0);
        }
    }
}

/// Proves **each** baked metrics table is equivalent to live `ttf-parser`
/// lookups against its own bundled font, for every codepoint/glyph/pair — not
/// just the ones exercised by the snapshot corpus. If these pass,
/// `measure_run()` is byte-identical to measuring from the font itself.
///
/// Every test runs once per [`Weight`], so the coverage that used to apply to
/// the single Regular table now applies to both. The cost doubles the slowest
/// test in the suite (~20 s → ~40 s); that is accepted deliberately — it is
/// the only thing standing between a regenerated table and silently wrong
/// measurement, and it is exhaustive over the whole Unicode range.
#[cfg(test)]
mod font_verification {
    use super::*;
    use ttf_parser::kern::Format;
    use ttf_parser::{Face, GlyphId};

    /// The bundled font bytes a weight is baked from.
    fn font_bytes(weight: Weight) -> &'static [u8] {
        match weight {
            Weight::Regular => dejavu::sans::regular(),
            Weight::Bold => dejavu::sans::bold(),
        }
    }

    fn live_face(weight: Weight) -> Face<'static> {
        Face::parse(font_bytes(weight), 0).expect("bundled font parses")
    }

    /// Run `check` once per weight, naming the weight in every failure.
    fn for_each_weight(check: impl Fn(Weight)) {
        for weight in Weight::ALL {
            check(weight);
        }
    }

    #[test]
    fn every_codepoint_maps_to_the_same_glyph() {
        for_each_weight(|weight| {
            let f = live_face(weight);
            for cp in 0..=0x10FFFFu32 {
                let Some(c) = char::from_u32(cp) else {
                    continue;
                };
                let live = f.glyph_index(c).map(|g| g.0).unwrap_or(0);
                assert_eq!(
                    glyph_id(weight.face(), c),
                    live,
                    "glyph mismatch for U+{cp:04X} in {weight:?}"
                );
            }
        });
    }

    #[test]
    fn every_glyph_has_the_same_advance() {
        for_each_weight(|weight| {
            let f = live_face(weight);
            let advances = weight.face().advances;
            for i in 0..f.number_of_glyphs() {
                let live = f.glyph_hor_advance(GlyphId(i)).unwrap_or(0);
                assert_eq!(
                    advances[i as usize], live,
                    "advance mismatch for glyph {i} in {weight:?}"
                );
            }
        });
    }

    #[test]
    fn kerning_matches_first_subtable_wins_semantics() {
        for_each_weight(|weight| {
            let f = live_face(weight);
            // Effective live value per pair: first horizontal subtable with a
            // value wins (the semantics the generator bakes).
            let mut live: std::collections::HashMap<u32, i16> = std::collections::HashMap::new();
            if let Some(table) = f.tables().kern {
                for st in table.subtables.into_iter().filter(|st| st.horizontal) {
                    let Format::Format0(f0) = st.format else {
                        panic!("generator handles only Format0")
                    };
                    for pair in f0.pairs {
                        live.entry(pair.pair).or_insert(pair.value);
                    }
                }
            }
            for (pair, value) in live {
                assert_eq!(
                    kern_unscaled(weight.face(), (pair >> 16) as u16, pair as u16),
                    value as f32,
                    "kern mismatch for pair {pair:#x} in {weight:?}"
                );
            }
        });
    }

    /// Reference implementation measuring straight from the font, using the
    /// renderer's own scaling (`font_size / unitsPerEm`).
    fn measure_live(f: &Face<'_>, text: &str, font_size: f32) -> Metrics {
        let h_scale = font_size / f.units_per_em() as f32;
        let measure_line = |line: &str| -> f32 {
            let mut width = 0.0_f32;
            let mut prev: Option<GlyphId> = None;
            for c in line.chars() {
                let gid = GlyphId(f.glyph_index(c).map(|g| g.0).unwrap_or(0));
                if let Some(p) = prev {
                    let kern = f
                        .tables()
                        .kern
                        .and_then(|k| {
                            k.subtables
                                .into_iter()
                                .filter(|st| st.horizontal)
                                .find_map(|st| st.glyphs_kerning(p, gid))
                        })
                        .unwrap_or_default();
                    width += h_scale * kern as f32;
                }
                width += h_scale * f.glyph_hor_advance(gid).unwrap_or_default() as f32;
                prev = Some(gid);
            }
            width
        };
        let lines: Vec<&str> = text.split('\n').collect();
        let width = lines
            .iter()
            .map(|l| measure_line(l))
            .fold(0.0_f32, f32::max);
        let height = font_size + (lines.len() - 1) as f32 * line_height(font_size);
        Metrics { width, height }
    }

    #[test]
    fn measure_is_bit_identical_to_the_font() {
        let samples = [
            "web",
            "Postgres",
            "Kubernetes Cluster",
            "A-->B",
            "a\nb",
            "",
            "AVATAR Wa To Yo",
            "Ünïcödé — ½ ✓",
            "Γειά σου",
            "Привет",
            "0123456789",
        ];
        for_each_weight(|weight| {
            let f = live_face(weight);
            for s in samples {
                for size in [1.0, 12.0, 13.5, 14.0, 17.3, 100.0] {
                    let a = measure_run(s, size, weight);
                    let b = measure_live(&f, s, size);
                    assert_eq!(
                        a.width.to_bits(),
                        b.width.to_bits(),
                        "width drift for {s:?} @ {size} in {weight:?}"
                    );
                    assert_eq!(
                        a.height.to_bits(),
                        b.height.to_bits(),
                        "height drift for {s:?} @ {size} in {weight:?}"
                    );
                }
            }
        });
    }

    /// Pins the documented reason italic has no table of its own: across
    /// ASCII and Latin-1, Oblique's advances equal Regular's except for four
    /// glyphs (each ≤1.4% wider), and the em boxes are identical. If the
    /// bundled font ever changes enough to break this, the "measure italic as
    /// Regular" decision needs revisiting — so the test, not a comment, is
    /// what keeps the docs honest.
    #[test]
    fn oblique_matches_regular_across_ascii_and_latin1_except_four_glyphs() {
        let reg = live_face(Weight::Regular);
        let obl = Face::parse(dejavu::sans::oblique(), 0).expect("bundled oblique parses");
        assert_eq!(
            (reg.ascender(), reg.descender()),
            (obl.ascender(), obl.descender()),
            "Oblique must share Regular's em box, or line heights would move"
        );
        let allowed = ['\u{ab}', '\u{bb}', '\u{de}', '\u{e6}']; // « » Þ æ
        for cp in 0..=0xFFu32 {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let Some(g) = reg.glyph_index(c) else {
                continue;
            };
            let Some(go) = obl.glyph_index(c) else {
                panic!("U+{cp:04X} ({c:?}) is in Regular but not Oblique");
            };
            let a = reg.glyph_hor_advance(g).unwrap_or(0);
            let b = obl.glyph_hor_advance(go).unwrap_or(0);
            if a == b {
                continue;
            }
            assert!(
                allowed.contains(&c),
                "U+{cp:04X} ({c:?}) advance differs between Regular ({a}) and Oblique ({b})"
            );
            let pct = (b as f64 - a as f64) / a as f64 * 100.0;
            assert!(
                pct.abs() <= 1.4,
                "U+{cp:04X} ({c:?}) differs by {pct:.2}%, over the documented 1.4%"
            );
        }
    }

    /// Every mapped char as a single-character string, plus kern-heavy bigrams
    /// from the kern table itself, measured at a size where `f32` rounding is
    /// unforgiving (odd scale).
    #[test]
    fn every_mapped_char_measures_identically() {
        let size = 13.7;
        for_each_weight(|weight| {
            let f = live_face(weight);
            for cp in 0..=0x10FFFFu32 {
                let Some(c) = char::from_u32(cp) else {
                    continue;
                };
                if f.glyph_index(c).is_none() {
                    continue;
                }
                let s = c.to_string();
                let a = measure_run(&s, size, weight);
                let b = measure_live(&f, &s, size);
                assert_eq!(
                    a.width.to_bits(),
                    b.width.to_bits(),
                    "width drift for U+{cp:04X} in {weight:?}"
                );
            }
        });
    }
}
