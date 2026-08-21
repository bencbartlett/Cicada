//! The `text_outlines` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Polyline};
use cicada_core::spatial::{Plane, Point};
use cicada_geom::frame::orthonormal;
use cicada_geom::text::layout;
use cicada_macros::{Ports, node};

use super::support::{bundled_font, outline_vertex_bound};
use crate::{checked_size, red};

/// Inputs for [`text_outlines`].
#[derive(Ports, Clone, Debug)]
pub struct TextOutlinesIn {
    /// The text to set; `\n` starts the next line.
    pub text: String,
    /// Capital-letter height (cap height) in document units.
    #[port(dimension = length)]
    pub size: f64,
    /// The frame: text runs along +x from the origin, baseline on the x
    /// axis, glyphs rising along +y.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// A bundled font name.
    #[port(default = "DejaVu Sans Bold")]
    pub font: String,
    /// Chords per bézier span when flattening the outlines.
    #[port(default = 8)]
    pub segments: i64,
    /// Baseline-to-baseline distance between lines, in multiples of `size`.
    #[port(default = 1.35)]
    pub line_gap: f64,
}

/// Text Outlines — glyph contours as closed polylines (béziers flattened
/// to `segments` per curve span), laid out left-to-right from the plane
/// origin along +x with the baseline on the x axis; `size` is the
/// capital-letter height; a newline in `text` stacks lines downward by
/// `line_gap` × `size`, left-aligned. Fonts are bundled in the binary
/// (reproducibility); the spike bundles `DejaVu Sans Bold` only. One curve
/// per contour, in text order then the font's contour order; whitespace
/// advances the pen and yields no curve.
///
/// # Returns
///
/// One closed polyline per glyph contour, in text order.
///
/// # Panics
///
/// Panics when the font is not bundled (the message lists the bundled
/// names), `size` is not above tolerance, a glyph is missing from the font
/// (names the character), `segments < 1`, the plane is degenerate, or the
/// outline vertices the text would flatten to (its bézier spans ×
/// `segments`, bounded from a two-chord pass) would be above the shared
/// ceilings (2^24 slots, or 1 GiB — the message names the count and the
/// ceiling that bit).
///
/// # Examples
///
/// ```cic
/// glyphs = text_outlines(text="A12", size=5.0)
/// ```
#[node(
    category = "Output, display & export",
    tier = "S",
    version = 1, gh = none,
    uses_tolerance
)]
#[must_use]
pub fn text_outlines(config: &ProjectConfig, input: TextOutlinesIn) -> Vec<Closed<Curve>> {
    let font = bundled_font(&input.font);
    let frame = red(orthonormal(&input.plane, config.tol()));
    // The vertex count is a PRODUCT of the text's spans and `segments`, so
    // it is checked as the derived size before the layout allocates it
    // (the layout keeps the floor, `segments < 1`).
    if input.segments >= 1 {
        let _ = checked_size(
            "text_outlines",
            "outline vertices",
            outline_vertex_bound(font, &input.text, input.segments),
            size_of::<Point>(),
        );
    }
    let glyphs = red(layout(
        font,
        &input.text,
        input.size,
        input.segments,
        input.line_gap,
        config.tol(),
    ));
    glyphs
        .iter()
        .flat_map(|glyph| glyph.contours.iter())
        .map(|contour| {
            Closed(Curve::Polyline(Polyline {
                vertices: contour
                    .points
                    .iter()
                    .map(|p| frame.point_at(p.x, p.y))
                    .collect(),
                closed: true,
            }))
        })
        .collect()
}

// The layout tests that compare outlines against solids live in
// `text_solids.rs`.
#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, List, ValueData};
    use glam::DVec3;

    use super::*;
    use crate::output::support::testing::{config, outlines, polyline_vertices};

    #[test]
    #[should_panic(expected = "bundled fonts: \"DejaVu Sans Bold\"")]
    fn unknown_font_is_red_with_the_bundled_list() {
        let _ = text_outlines(
            &config(),
            TextOutlinesIn {
                text: "A".to_owned(),
                size: 5.0,
                plane: Plane::world_xy(),
                font: "Comic Sans".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        );
    }

    #[test]
    #[should_panic(expected = "size")]
    fn zero_size_is_red() {
        let _ = outlines("A", 0.0);
    }

    #[test]
    #[should_panic(expected = "(U+1F41B) in the font")]
    fn missing_glyph_names_the_character() {
        let _ = outlines("A\u{1f41b}", 5.0);
    }

    // One chord past the slot ceiling for this text: the two-chord count
    // of `A` times `segments` crosses 2^24 — red with the bound in the
    // message, before a single outline is laid out at that density.
    #[test]
    fn one_chord_past_the_vertex_ceiling_is_refused_not_allocated() {
        let font = bundled_font("DejaVu Sans Bold");
        let spans = outline_vertex_bound(font, "A", 1); // the two-chord count
        let segments =
            i64::try_from(u128::from(crate::MAX_SLOTS.unsigned_abs()) / spans + 1).unwrap();
        let would_be = spans * u128::from(segments.unsigned_abs());
        assert!(would_be > u128::from(crate::MAX_SLOTS.unsigned_abs()));
        let panic = std::panic::catch_unwind(|| {
            text_outlines(
                &config(),
                TextOutlinesIn {
                    text: "A".to_owned(),
                    size: 5.0,
                    plane: Plane::world_xy(),
                    font: "DejaVu Sans Bold".to_owned(),
                    segments,
                    line_gap: 1.35,
                },
            )
        })
        .expect_err("one chord past the ceiling refuses");
        assert_eq!(
            *panic.downcast_ref::<String>().unwrap(),
            format!(
                "text_outlines: outline vertices would be {would_be} — above the 16777216 (2^24) \
                 slot ceiling of one node output"
            )
        );
        // One chord fewer is under the ceiling: the boundary is exact.
        assert!(spans * u128::from((segments - 1).unsigned_abs()) <= 1 << 24);
    }

    // The guard's bound is a bound: what the node really produces at the
    // default density never exceeds it (two lines, a hole-bearing glyph,
    // whitespace).
    #[test]
    fn the_vertex_bound_covers_what_the_layout_produces() {
        let font = bundled_font("DejaVu Sans Bold");
        let text = "A12 B\n@g%";
        let produced: usize = text_outlines(
            &config(),
            TextOutlinesIn {
                text: text.to_owned(),
                size: 5.0,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        )
        .iter()
        .map(|curve| polyline_vertices(curve).len())
        .sum();
        let bound = outline_vertex_bound(font, text, 8);
        assert!(
            u128::try_from(produced).unwrap() <= bound,
            "{produced} vertices produced, bound {bound}"
        );
        assert!(
            u128::try_from(produced).unwrap() * 4 >= bound,
            "the bound is within 4× of the truth: {produced} vs {bound}"
        );
    }

    #[test]
    #[should_panic(expected = "segments")]
    fn zero_segments_is_red() {
        let _ = text_outlines(
            &config(),
            TextOutlinesIn {
                text: "A".to_owned(),
                size: 5.0,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments: 0,
                line_gap: 1.35,
            },
        );
    }

    #[test]
    fn segments_scale_the_vertex_count() {
        let coarse = outlines("O", 5.0);
        let fine = text_outlines(
            &config(),
            TextOutlinesIn {
                text: "O".to_owned(),
                size: 5.0,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments: 16,
                line_gap: 1.35,
            },
        );
        let count = |curves: &[Closed<Curve>]| -> usize {
            curves.iter().map(|c| polyline_vertices(c).len()).sum()
        };
        assert!(
            count(&fine) > count(&coarse) * 3 / 2,
            "{} vs {}",
            count(&fine),
            count(&coarse)
        );
    }

    proptest::proptest! {
        // Outlines: translating the plane translates every vertex exactly.
        #[test]
        fn property_outlines_follow_the_plane_origin(
            ox in -100.0f64..100.0, oy in -100.0f64..100.0, oz in -100.0f64..100.0,
        ) {
            let at_origin = outlines("Hi", 3.0);
            let moved = text_outlines(
                &config(),
                TextOutlinesIn {
                    text: "Hi".to_owned(),
                    size: 3.0,
                    plane: Plane { origin: Point::new(ox, oy, oz), ..Plane::world_xy() },
                    font: "DejaVu Sans Bold".to_owned(),
                    segments: 8,
                    line_gap: 1.35,
                },
            );
            proptest::prop_assert_eq!(at_origin.len(), moved.len());
            for (a, b) in at_origin.iter().zip(&moved) {
                for (pa, pb) in polyline_vertices(a).iter().zip(polyline_vertices(b)) {
                    let d = pb.0 - pa.0 - DVec3::new(ox, oy, oz);
                    proptest::prop_assert!(d.length() < 1e-9);
                }
            }
        }
    }

    // Golden hash: pure arithmetic throughout (flattening is polynomial
    // evaluation, layout is adds and one division), so this is
    // platform-stable like the box/extrude goldens.
    // Golden hashes: pure arithmetic throughout (flattening is polynomial
    // evaluation, layout is adds and one division), so these are
    // platform-stable like the box/extrude goldens.
    #[test]
    fn text_outlines_determinism_golden_hash() {
        let curves = outlines("A12\nb", 5.0);
        let slots = curves
            .into_iter()
            .map(|c| Some(HashedValue::new(ValueData::Curve(c.0)).unwrap()))
            .collect();
        let list = HashedValue::new(ValueData::List(List { axis: None, slots })).unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "de70e4947374da5e28d237ba7a2cee89b276f010951a6ea7be60f3b33ab85a53"
        );
    }
}
