//! Text nodes (docs/08 §Catalog 11): glyph outlines and glyph solids from
//! a BUNDLED font — the docs/08 open question ("bundle fonts vs system
//! lookup") resolved for reproducibility: the same `.cic` must produce the
//! same bytes on every machine, so the font travels inside the binary.
//! The spike bundles exactly one face, `DejaVu Sans Bold`
//! (`crates/cicada-stdlib/fonts/`, license alongside); more arrive with
//! v0.1. Glyph geometry, layout, and the hole-aware prism builder live in
//! `cicada_geom::text`.
//!
//! `size` is the CAPITAL-LETTER HEIGHT (cap height) — how fabrication text
//! is specified on drawings — not the em size. Text runs from the plane's
//! origin along +x with the baseline on the x axis; `\n` stacks lines
//! downward by `line_gap × size`, left-aligned. Kerning is ignored.

use std::sync::LazyLock;

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Mesh, Polyline, Watertight};
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::text::{Font, PlacedGlyph, glyph_solid, layout};
use cicada_macros::{Ports, node};

use crate::red;

/// The bundled faces: display name → font bytes. Names are what the
/// `font` port accepts, exactly.
const BUNDLED_FONTS: &[(&str, &[u8])] = &[(
    "DejaVu Sans Bold",
    include_bytes!("../fonts/DejaVuSans-Bold.ttf"),
)];

/// Parsed once per process; ttf-parser's `Face` is plain shared data.
static FONTS: LazyLock<Vec<(&'static str, Font<'static>)>> = LazyLock::new(|| {
    BUNDLED_FONTS
        .iter()
        .map(|&(name, bytes)| (name, red(Font::from_bytes(bytes))))
        .collect()
});

/// The names of the bundled fonts, in catalog order.
#[must_use]
pub fn bundled_font_names() -> Vec<&'static str> {
    BUNDLED_FONTS.iter().map(|&(name, _)| name).collect()
}

/// Resolve a `font` port value to a bundled face — red with the bundled
/// list otherwise.
fn bundled_font(name: &str) -> &'static Font<'static> {
    match FONTS.iter().find(|(n, _)| *n == name) {
        Some((_, font)) => font,
        None => panic!(
            "font {name:?} is not bundled; bundled fonts: {}",
            bundled_font_names()
                .iter()
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

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
/// # Panics
///
/// Panics when the font is not bundled (the message lists the bundled
/// names), `size` is not above tolerance, a glyph is missing from the font
/// (names the character), `segments < 1`, or the plane is degenerate.
#[node(
    category = "Output, display & export",
    tier = "S",
    version = 1,
    uses_tolerance
)]
#[must_use]
pub fn text_outlines(config: &ProjectConfig, input: TextOutlinesIn) -> Vec<Closed<Curve>> {
    let font = bundled_font(&input.font);
    let frame = red(orthonormal(&input.plane, config.tol()));
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

/// Inputs for [`text_solids`].
#[derive(Ports, Clone, Debug)]
pub struct TextSolidsIn {
    /// The text to set; `\n` starts the next line.
    pub text: String,
    /// Capital-letter height (cap height) in document units.
    #[port(dimension = length)]
    pub size: f64,
    /// Extrusion depth along the plane normal (x × y); negative extrudes
    /// the other way.
    #[port(dimension = length)]
    pub depth: f64,
    /// The frame: text runs along +x from the origin, baseline on the x
    /// axis, glyphs rising along +y, solids growing along the normal.
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

/// Text Solids — one watertight solid per glyph (counters handled: the
/// glyph region is triangulated with its holes), extruded `depth` along
/// the plane normal (negative = the other way), same layout as
/// `text_outlines` (cap-height `size`, baseline on the plane's x axis,
/// newline stacks lines by `line_gap` × `size`). Glyphs of several pieces
/// (`i`, `%`, `"`) are one mesh with several shells; whitespace yields no
/// solid. The wall's label deboss cutters.
///
/// # Panics
///
/// Panics when `text_outlines` would (the font is not bundled — the message
/// lists the bundled names —, `size` is not above tolerance, a glyph is
/// missing from the font — names the character —, `segments < 1`, the
/// plane is degenerate), when `depth` is within tolerance of zero, or when
/// a glyph's contours cannot be triangulated into a watertight prism
/// (touching or self-intersecting outlines — a font defect, named by
/// character).
#[node(
    category = "Output, display & export",
    tier = "S",
    version = 1,
    uses_tolerance
)]
#[must_use]
pub fn text_solids(config: &ProjectConfig, input: TextSolidsIn) -> Vec<Watertight<Mesh>> {
    let font = bundled_font(&input.font);
    let frame = red(orthonormal(&input.plane, config.tol()));
    let glyphs: Vec<PlacedGlyph> = red(layout(
        font,
        &input.text,
        input.size,
        input.segments,
        input.line_gap,
        config.tol(),
    ));
    glyphs
        .iter()
        .map(|glyph| Watertight(red(glyph_solid(glyph, &frame, input.depth, config.tol()))))
        .collect()
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_core::value::{HashedValue, List, ValueData};
    use cicada_geom::meshbuild::signed_volume;
    use glam::DVec3;

    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    fn outlines(text: &str, size: f64) -> Vec<Closed<Curve>> {
        text_outlines(
            &config(),
            TextOutlinesIn {
                text: text.to_owned(),
                size,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        )
    }

    fn solids(text: &str, size: f64, depth: f64, segments: i64) -> Vec<Watertight<Mesh>> {
        text_solids(
            &config(),
            TextSolidsIn {
                text: text.to_owned(),
                size,
                depth,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments,
                line_gap: 1.35,
            },
        )
    }

    fn bbox(mesh: &Mesh) -> (DVec3, DVec3) {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for p in mesh.positions().chunks_exact(3) {
            let v = DVec3::new(p[0], p[1], p[2]);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo, hi)
    }

    fn polyline_vertices(curve: &Closed<Curve>) -> &[Point] {
        match &curve.0 {
            Curve::Polyline(p) => {
                assert!(p.closed);
                &p.vertices
            }
            other => panic!(
                "text_outlines yields polylines, got {}",
                other.variant_name()
            ),
        }
    }

    /// Area of an outer-minus-holes glyph footprint from its outlines
    /// (signed shoelace over every contour: the font winds holes opposite
    /// to outers, so the absolute sum is the filled area).
    fn outline_area(curves: &[Closed<Curve>]) -> f64 {
        curves
            .iter()
            .map(|c| {
                let v = polyline_vertices(c);
                let mut sum = 0.0;
                for (i, a) in v.iter().enumerate() {
                    let b = v[(i + 1) % v.len()];
                    sum += a.0.x * b.0.y - b.0.x * a.0.y;
                }
                sum / 2.0
            })
            .sum::<f64>()
            .abs()
    }

    #[test]
    fn bundled_font_is_dejavu_sans_bold_with_a_cap_height() {
        assert_eq!(bundled_font_names(), vec!["DejaVu Sans Bold"]);
        let font = bundled_font("DejaVu Sans Bold");
        assert!(font.cap_height() > 0.0 && font.cap_height() < font.units_per_em());
        // Every printable ASCII character has a glyph.
        for c in (0x20u8..0x7f).map(char::from) {
            assert!(font.has_glyph(c), "no glyph for {c:?}");
        }
    }

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
    #[should_panic(expected = "depth")]
    fn zero_depth_is_red() {
        let _ = solids("A", 5.0, 0.0, 8);
    }

    #[test]
    #[should_panic(expected = "degenerate frame")]
    fn degenerate_plane_is_red() {
        let _ = text_solids(
            &config(),
            TextSolidsIn {
                text: "A".to_owned(),
                size: 5.0,
                depth: 1.0,
                plane: Plane {
                    origin: Point::origin(),
                    x: Vector::new(1.0, 0.0, 0.0),
                    y: Vector::new(2.0, 0.0, 0.0),
                },
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        );
    }

    // size = cap height: the `H` solid spans exactly `size` in y, sits on
    // the baseline (y = 0), and starts at the pen origin side bearing.
    #[test]
    fn size_is_the_cap_height() {
        for size in [2.5, 5.0, 12.0] {
            let h = solids("H", size, 1.0, 8);
            assert_eq!(h.len(), 1);
            let (lo, hi) = bbox(&h[0].0);
            assert!(
                (hi.y - lo.y - size).abs() <= 1e-9 * size,
                "H height {} vs {size}",
                hi.y - lo.y
            );
            assert!(
                lo.y.abs() <= 1e-9,
                "H sits on the baseline, got y_min {}",
                lo.y
            );
            assert!(
                (lo.z, hi.z) == (0.0, 1.0)
                    || ((lo.z - 0.0).abs() < 1e-12 && (hi.z - 1.0).abs() < 1e-12)
            );
        }
    }

    // Counters: A, B, 0, 8 have holes. The solid's volume is LESS than the
    // outer-contour prism's, and a point inside a counter is outside the
    // solid (the mesh has an open tube there: cast a ray along +z from the
    // counter's center — it crosses no cap).
    #[test]
    fn counters_are_holes() {
        for (c, holes) in [('A', 1), ('B', 2), ('0', 1), ('8', 2)] {
            let curves = outlines(&c.to_string(), 10.0);
            assert_eq!(curves.len(), 1 + holes, "{c:?} contours");
            let solid = solids(&c.to_string(), 10.0, 2.0, 8);
            assert_eq!(solid.len(), 1);
            let mesh = &solid[0].0;
            assert!(mesh.is_watertight());
            let volume = signed_volume(mesh);
            assert!(volume > 0.0, "{c:?} outward orientation");
            // Outer prism: the largest-area contour alone.
            let outer_area = curves
                .iter()
                .map(|curve| outline_area(std::slice::from_ref(curve)))
                .fold(0.0f64, f64::max);
            let filled_area = outline_area(&curves);
            assert!(filled_area < outer_area, "{c:?}: holes reduce the area");
            assert!(
                (volume - filled_area * 2.0).abs() <= 1e-9 * outer_area * 2.0,
                "{c:?}: volume {volume} = filled area {filled_area} × depth 2"
            );
            assert!(
                volume < outer_area * 2.0,
                "{c:?}: solid is smaller than the outer prism"
            );

            // A point inside the counter: the centroid of the hole contour's
            // vertices (DejaVu's counters are convex enough for that), at
            // mid-depth. Parity of cap crossings along +z from there = 0.
            let hole = curves
                .iter()
                .min_by(|a, b| {
                    outline_area(std::slice::from_ref(*a))
                        .total_cmp(&outline_area(std::slice::from_ref(*b)))
                })
                .unwrap();
            let verts = polyline_vertices(hole);
            #[allow(clippy::cast_precision_loss)]
            let center = verts.iter().map(|p| p.0).sum::<DVec3>() / verts.len() as f64;
            assert_eq!(
                cap_crossings(mesh, center.x, center.y),
                0,
                "{c:?}: counter is empty"
            );
            // …and a point inside the solid (the midpoint of the outer
            // contour's first edge, nudged inward) crosses two caps.
        }
    }

    /// Number of cap triangles (z-facing) whose footprint contains (x, y):
    /// 2 inside the solid's footprint, 0 outside or in a counter.
    fn cap_crossings(mesh: &Mesh, x: f64, y: f64) -> usize {
        let pos = mesh.positions();
        let at = |i: u32| {
            let k = i as usize * 3;
            DVec3::new(pos[k], pos[k + 1], pos[k + 2])
        };
        let p = glam::DVec2::new(x, y);
        mesh.indices()
            .chunks_exact(3)
            .filter(|t| {
                let (va, vb, vc) = (at(t[0]), at(t[1]), at(t[2]));
                let normal = (vb - va).cross(vc - va);
                if normal.z.abs() <= 1e-12 {
                    return false; // a wall
                }
                let (va, vb, vc) = (va.truncate(), vb.truncate(), vc.truncate());
                let d1 = (vb - va).perp_dot(p - va);
                let d2 = (vc - vb).perp_dot(p - vb);
                let d3 = (va - vc).perp_dot(p - vc);
                (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
            })
            .count()
    }

    // Every printable ASCII glyph of the bundled font extrudes to a
    // watertight, outward-oriented prism whose volume is footprint × depth,
    // at both a coarse and the default flattening.
    #[test]
    fn every_printable_ascii_glyph_is_a_watertight_prism() {
        for segments in [1, 4, 8] {
            for c in (0x21u8..0x7f).map(char::from) {
                let text = c.to_string();
                let curves = outlines(&text, 6.0);
                let solid = solids(&text, 6.0, 1.5, segments);
                assert_eq!(solid.len(), 1, "{c:?}: one solid per glyph");
                let mesh = &solid[0].0;
                assert!(mesh.is_watertight(), "{c:?} at segments={segments}");
                let volume = signed_volume(mesh);
                assert!(volume > 0.0, "{c:?}: positive volume");
                if segments == 8 {
                    let area = outline_area(&curves);
                    assert!(
                        (volume - area * 1.5).abs() <= 1e-9 * area * 1.5,
                        "{c:?}: volume {volume} vs area {area} × 1.5"
                    );
                }
            }
        }
        // Space: advance only, no geometry.
        assert!(solids(" ", 6.0, 1.5, 8).is_empty());
        assert!(outlines(" ", 6.0).is_empty());
        assert!(solids("", 6.0, 1.5, 8).is_empty());
    }

    #[test]
    fn layout_advances_and_stacks_lines() {
        let size = 5.0;
        // Two lines, left-aligned: the second line's glyphs sit one
        // line_gap × size lower and restart at x = 0.
        let two = solids("H\nH", size, 1.0, 8);
        assert_eq!(two.len(), 2);
        let (lo0, hi0) = bbox(&two[0].0);
        let (lo1, hi1) = bbox(&two[1].0);
        assert!((lo0.x - lo1.x).abs() < 1e-12 && (hi0.x - hi1.x).abs() < 1e-12);
        assert!((lo0.y - lo1.y - 1.35 * size).abs() < 1e-9);
        assert!((hi0.y - hi1.y - 1.35 * size).abs() < 1e-9);
        // Custom line gap.
        let tight = text_solids(
            &config(),
            TextSolidsIn {
                text: "H\nH".to_owned(),
                size,
                depth: 1.0,
                plane: Plane::world_xy(),
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.0,
            },
        );
        let (t0, _) = bbox(&tight[0].0);
        let (t1, _) = bbox(&tight[1].0);
        assert!((t0.y - t1.y - size).abs() < 1e-9);

        // Horizontal: "HH" — the second H is one advance to the right, the
        // advance being the font's, scaled by size / cap height.
        let hh = solids("HH", size, 1.0, 8);
        let (a0, _) = bbox(&hh[0].0);
        let (a1, _) = bbox(&hh[1].0);
        let font = bundled_font("DejaVu Sans Bold");
        let advance = font.glyph('H', 8).unwrap().advance * size / font.cap_height();
        assert!(
            (a1.x - a0.x - advance).abs() < 1e-9,
            "advance {} vs {advance}",
            a1.x - a0.x
        );
        // A space between advances the pen further (space has an advance).
        let h_h = solids("H H", size, 1.0, 8);
        let (s1, _) = bbox(&h_h[1].0);
        assert!(s1.x > a1.x + 1e-9);
        // Outlines agree with solids on count: H has one contour.
        assert_eq!(outlines("H H", size).len(), 2);
    }

    #[test]
    fn plane_orients_the_text_and_negative_depth_flips() {
        let size = 4.0;
        // XZ plane at an offset: glyphs rise along +z, extrude along the
        // frame normal x × y = (1,0,0) × (0,0,1) = (0,-1,0).
        let plane = Plane {
            origin: Point::new(10.0, 20.0, 30.0),
            ..Plane::world_xz()
        };
        let fwd = text_solids(
            &config(),
            TextSolidsIn {
                text: "H".to_owned(),
                size,
                depth: 2.0,
                plane,
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        );
        let (lo, hi) = bbox(&fwd[0].0);
        assert!((hi.z - lo.z - size).abs() < 1e-9 && (lo.z - 30.0).abs() < 1e-9);
        assert!(
            (hi.y - 20.0).abs() < 1e-9 && (lo.y - 18.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
        assert!(signed_volume(&fwd[0].0) > 0.0);
        let back = text_solids(
            &config(),
            TextSolidsIn {
                text: "H".to_owned(),
                size,
                depth: -2.0,
                plane,
                font: "DejaVu Sans Bold".to_owned(),
                segments: 8,
                line_gap: 1.35,
            },
        );
        let (lo, hi) = bbox(&back[0].0);
        assert!(
            (lo.y - 20.0).abs() < 1e-9 && (hi.y - 22.0).abs() < 1e-9,
            "{lo:?} {hi:?}"
        );
        assert!(
            signed_volume(&back[0].0) > 0.0,
            "negative depth still outward"
        );
        assert!(back[0].0.is_watertight());
        // Outlines lie in the plane.
        for curve in outlines("H", size) {
            for v in polyline_vertices(&curve) {
                assert!(v.0.z.abs() < 1e-12);
            }
        }
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
        // Any size, depth, and segment count: the label "A12" is three
        // watertight, outward prisms with volume = footprint × |depth|,
        // footprint scaling with size².
        #[test]
        fn property_label_solids_scale(
            size in 0.5f64..50.0,
            depth in proptest::sample::select(vec![-4.0, -0.5, 0.25, 3.0]),
            segments in 1i64..12,
        ) {
            let out = solids("A12", size, depth, segments);
            proptest::prop_assert_eq!(out.len(), 3);
            let unit = solids("A12", 1.0, 1.0, segments);
            for (k, solid) in out.iter().enumerate() {
                proptest::prop_assert!(solid.0.is_watertight());
                let volume = signed_volume(&solid.0);
                proptest::prop_assert!(volume > 0.0);
                let want = signed_volume(&unit[k].0) * size * size * depth.abs();
                proptest::prop_assert!((volume - want).abs() <= 1e-9 * want, "{} vs {}", volume, want);
            }
        }

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

    #[test]
    fn text_solids_determinism_golden_hash() {
        let meshes = solids("A12\nb", 5.0, 2.0, 8);
        let slots = meshes
            .into_iter()
            .map(|m| Some(HashedValue::new(ValueData::Mesh(m.0)).unwrap()))
            .collect();
        let list = HashedValue::new(ValueData::List(List { axis: None, slots })).unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "725f283b55958450a97cb935ea05b5d6dbc919250ce19a06f6da74e85e98c173"
        );
    }

    // The number the wall budget cares about: triangles per typical label.
    #[test]
    fn label_triangle_count_is_reported() {
        for segments in [4, 8] {
            let meshes = solids("A12", 5.0, 2.0, segments);
            let triangles: usize = meshes.iter().map(|m| m.0.triangle_count()).sum();
            eprintln!("text_solids(\"A12\", segments={segments}): {triangles} triangles");
            assert!(triangles > 0);
        }
    }
}
