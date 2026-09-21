//! Precise text measurement via `ab_glyph`, using the bundled DejaVu Sans
//! Regular provided by the `dejavu` crate.
//!
//! Node labels must be sized before the boxes can be placed (milestone 2).
//! We measure the *layout width* (summed glyph advance, with kerning) and the
//! em height (ascent − descent) at a fixed pixel size, deterministically,
//! from a single embedded font so that output is reproducible across machines.

use ab_glyph::{Font, FontArc, GlyphId, ScaleFont};
use std::sync::OnceLock;

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

/// Lazily parsed, statically cached copy of the bundled DejaVu Sans Regular.
fn font() -> &'static FontArc {
    static FONT: OnceLock<FontArc> = OnceLock::new();
    FONT.get_or_init(|| {
        FontArc::try_from_slice(dejavu::sans::regular())
            .expect("bundled DejaVu Sans Regular must parse as a valid font")
    })
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
    let f = font();
    let sf = f.as_scaled(font_size);
    let measure_line = |line: &str| -> f32 {
        let mut width = 0.0_f32;
        let mut prev: Option<GlyphId> = None;
        for c in line.chars() {
            let gid = sf.glyph_id(c);
            if let Some(p) = prev {
                width += sf.kern(p, gid);
            }
            width += sf.h_advance(gid);
            prev = Some(gid);
        }
        width
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines.iter().map(|l| measure_line(l)).fold(0.0_f32, f32::max);
    let height = sf.height() + (lines.len() - 1) as f32 * line_height(font_size);
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

#[cfg(test)]
mod tmp_probe {
    use super::*;
    #[test]
    fn tmp_print() {
        for t in ["Group", "Kubernetes Cluster", "A Very Long Subgraph Title Indeed"] {
            println!("{t:?} @12 -> {:.2}   @14 -> {:.2}", measure(t, 12.0).width, measure(t, 14.0).width);
        }
    }
}
