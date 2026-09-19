//! Precise text measurement via `ab_glyph`, using the bundled DejaVu Sans
//! Regular provided by the `dejavu` crate.
//!
//! Node labels must be sized before the boxes can be placed (milestone 2).
//! We measure the *layout width* (summed glyph advance, with kerning) and the
//! em height (ascent − descent) at a fixed pixel size, deterministically,
//! from a single embedded font so that output is reproducible across machines.

use ab_glyph::{Font, FontArc, GlyphId, ScaleFont};
use std::sync::OnceLock;

/// Measurement of a single-line string at a given font size, in pixels.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// Summed horizontal advance (the layout width, including the trailing
    /// advance of the last glyph).
    pub width: f32,
    /// Em height: ascent − descent. For a uniform pixel scale this equals the
    /// `font_size` passed in.
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

/// Measure `text` rendered at `font_size` pixels (a single line).
///
/// Missing glyphs map to the font's `.notdef` (advance may be nonzero), which
/// is fine for v0 — labels are expected to be covered by DejaVu Sans (Latin,
/// digits, punctuation, common symbols).
pub fn measure(text: &str, font_size: f32) -> Metrics {
    let f = font();
    let sf = f.as_scaled(font_size);
    let mut width = 0.0_f32;
    let mut prev: Option<GlyphId> = None;
    for c in text.chars() {
        let gid = sf.glyph_id(c);
        if let Some(p) = prev {
            width += sf.kern(p, gid);
        }
        width += sf.h_advance(gid);
        prev = Some(gid);
    }
    Metrics {
        width,
        height: sf.height(),
    }
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
        for s in ["web", "Postgres", "Kubernetes Cluster", "A-->B"] {
            assert_eq!(measure(s, 14.0).width, measure(s, 14.0).width);
        }
    }
}
