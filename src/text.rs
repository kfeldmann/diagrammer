//! Precise text measurement against the baked DejaVu Sans Regular metrics in
//! [`crate::metrics_table`].
//!
//! Node labels must be sized before the boxes can be placed (milestone 2).
//! We measure the *layout width* (summed glyph advance, with kerning) and the
//! em height (ascent − descent) at a fixed pixel size, deterministically,
//! from a single font's baked metrics so that output is reproducible across
//! machines. The metrics table is generated from the DejaVu Sans Regular
//! embedded via the `dejavu` crate (`cargo run -p gen-metrics-table`)
//! and verified against it by the `font_verification` tests.

use crate::metrics_table as mt;

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

/// Pixels per font unit at `font_size` pixels. The em box is the font's
/// `ascent − descent` (not `units_per_em`) — matching the historical `ab_glyph`
/// `PxScaleFont` scaling exactly, so measurements are unchanged.
fn h_scale_factor(font_size: f32) -> f32 {
    font_size / (mt::ASCENT as f32 - mt::DESCENT as f32)
}

/// Map `c` to a glyph id via the baked cmap runs; unmapped chars map to
/// `.notdef` (id 0), whose advance may be nonzero — matching the documented
/// v0 behavior.
fn glyph_id(c: char) -> u16 {
    let cp = c as u32;
    match mt::CHAR_RUNS.binary_search_by_key(&cp, |r| r.0) {
        // Exact hit on a run start.
        Ok(i) => mt::CHAR_RUNS[i].1,
        // Inside the run ending just before this insertion point.
        Err(0) => 0,
        Err(i) => {
            let (start, gid, len) = mt::CHAR_RUNS[i - 1];
            if cp < start + len as u32 {
                gid + (cp - start) as u16
            } else {
                0
            }
        }
    }
}

/// Unscaled horizontal advance of `id` in font units (unknown id → 0).
fn h_advance_unscaled(id: u16) -> f32 {
    mt::ADVANCES.get(id as usize).copied().unwrap_or_default() as f32
}

/// Unscaled kerning between `first` and `second` in font units (none → 0).
fn kern_unscaled(first: u16, second: u16) -> f32 {
    let pair = (first as u32) << 16 | second as u32;
    mt::KERN
        .binary_search_by_key(&pair, |k| k.0)
        .map(|i| mt::KERN[i].1)
        .unwrap_or_default() as f32
}

/// Measure `text` rendered at `font_size` pixels.
///
/// A `\n` in `text` marks a line break: `width` is the widest line's layout
/// width and `height` spans every line (first line's em height plus one
/// [`line_height`] per additional line), matching how the renderer stacks
/// `<tspan>` lines.
///
/// Missing glyphs map to the font's `.notdef` (advance may be nonzero), which
/// is fine for v0 — labels are expected to be covered by DejaVu Sans (Latin,
/// digits, punctuation, common symbols).
pub fn measure(text: &str, font_size: f32) -> Metrics {
    let h_scale = h_scale_factor(font_size);
    let measure_line = |line: &str| -> f32 {
        let mut width = 0.0_f32;
        let mut prev: Option<u16> = None;
        for c in line.chars() {
            let gid = glyph_id(c);
            if let Some(p) = prev {
                width += h_scale * kern_unscaled(p, gid);
            }
            width += h_scale * h_advance_unscaled(gid);
            prev = Some(gid);
        }
        width
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines.iter().map(|l| measure_line(l)).fold(0.0_f32, f32::max);
    let height = font_size + (lines.len() - 1) as f32 * line_height(font_size);
    Metrics { width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_text_is_wider() {
        let a = measure("API", 14.0);
        let b = measure("API Server 1", 14.0);
        assert!(b.width > a.width, "{} should be wider than {}", b.width, a.width);
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
        assert!((m.width - wide).abs() < 1e-4, "width should be the widest line");
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
}

/// Proves the baked metrics table is equivalent to live `ttf-parser` lookups
/// against the bundled DejaVu Sans Regular, for every codepoint/glyph/pair —
/// not just the ones exercised by the snapshot corpus. If these pass,
/// `measure()` is byte-identical to measuring from the font itself.
#[cfg(test)]
mod font_verification {
    use super::*;
    use ttf_parser::kern::Format;
    use ttf_parser::{Face, GlyphId};

    fn face() -> Face<'static> {
        Face::parse(dejavu::sans::regular(), 0).expect("bundled font parses")
    }

    #[test]
    fn every_codepoint_maps_to_the_same_glyph() {
        let f = face();
        for cp in 0..=0x10FFFFu32 {
            let Some(c) = char::from_u32(cp) else { continue };
            let live = f.glyph_index(c).map(|g| g.0).unwrap_or(0);
            assert_eq!(glyph_id(c), live, "glyph mismatch for U+{cp:04X}");
        }
    }

    #[test]
    fn every_glyph_has_the_same_advance() {
        let f = face();
        for i in 0..f.number_of_glyphs() {
            let live = f.glyph_hor_advance(GlyphId(i)).unwrap_or(0);
            assert_eq!(mt::ADVANCES[i as usize], live, "advance mismatch for glyph {i}");
        }
    }

    #[test]
    fn kerning_matches_first_subtable_wins_semantics() {
        let f = face();
        // Effective live value per pair: first horizontal subtable with a
        // value wins (the semantics the generator bakes).
        let mut live: std::collections::HashMap<u32, i16> = std::collections::HashMap::new();
        if let Some(table) = f.tables().kern {
            for st in table.subtables.into_iter().filter(|st| st.horizontal) {
                let Format::Format0(f0) = st.format else { panic!("generator handles only Format0") };
                for pair in f0.pairs {
                    live.entry(pair.pair).or_insert(pair.value);
                }
            }
        }
        for (pair, value) in live {
            assert_eq!(
                kern_unscaled((pair >> 16) as u16, pair as u16),
                value as f32,
                "kern mismatch for pair {pair:#x}"
            );
        }
    }

    /// Reference implementation measuring straight from the font, with the
    /// exact historical `ab_glyph` scaling.
    fn measure_live(f: &Face<'_>, text: &str, font_size: f32) -> Metrics {
        let h_scale =
            font_size / (f.ascender() as f32 - f.descender() as f32);
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
        let width = lines.iter().map(|l| measure_line(l)).fold(0.0_f32, f32::max);
        let height = font_size + (lines.len() - 1) as f32 * line_height(font_size);
        Metrics { width, height }
    }

    #[test]
    fn measure_is_bit_identical_to_the_font() {
        let f = face();
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
        for s in samples {
            for size in [1.0, 12.0, 13.5, 14.0, 17.3, 100.0] {
                let a = measure(s, size);
                let b = measure_live(&f, s, size);
                assert_eq!(a.width.to_bits(), b.width.to_bits(), "width drift for {s:?} @ {size}");
                assert_eq!(a.height.to_bits(), b.height.to_bits(), "height drift for {s:?} @ {size}");
            }
        }
    }

    /// Every mapped char as a single-character string, plus kern-heavy bigrams
    /// from the kern table itself, measured at a size where `f32` rounding is
    /// unforgiving (odd scale).
    #[test]
    fn every_mapped_char_measures_identically() {
        let f = face();
        let size = 13.7;
        for cp in 0..=0x10FFFFu32 {
            let Some(c) = char::from_u32(cp) else { continue };
            if f.glyph_index(c).is_none() {
                continue;
            }
            let s = c.to_string();
            let a = measure(&s, size);
            let b = measure_live(&f, &s, size);
            assert_eq!(a.width.to_bits(), b.width.to_bits(), "width drift for U+{cp:04X}");
        }
    }
}