//! The bundled-font table shared by the text nodes, the outline-vertex
//! bound their allocation guard checks, plus their test helpers.

use std::sync::LazyLock;

use cicada_geom::text::Font;

use crate::red;

/// The bundled faces: display name → font bytes. Names are what the
/// `font` port accepts, exactly.
const BUNDLED_FONTS: &[(&str, &[u8])] = &[(
    "DejaVu Sans Bold",
    include_bytes!("../../fonts/DejaVuSans-Bold.ttf"),
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
pub(crate) fn bundled_font(name: &str) -> &'static Font<'static> {
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

/// An upper bound on the outline vertices `text` flattens to at `segments`
/// chords per bézier span, from the spans the font draws each glyph with
/// ([`Font::outline_spans`], no flattening): a contour start or a line
/// span is at most one vertex at any density, a bézier span at most
/// `segments` — the bound holds by construction of the flattener (it only
/// ever drops vertices), so a line-only glyph costs its line count however
/// fine the chords, and a contour that is one closed bézier loop is
/// counted like any other span. The guard is the text's, not a per-glyph
/// constant: the heaviest bundled glyph has hundreds of spans and a
/// typical one about twenty, and a constant safe for every character would
/// refuse honest paragraphs. Only characters the face maps are counted —
/// `layout` names a missing glyph itself, in its own order — and a newline
/// is layout, not a glyph.
pub(crate) fn outline_vertex_bound(font: &Font<'_>, text: &str, segments: i64) -> u128 {
    let segments = segments.unsigned_abs();
    text.chars()
        .filter(|&c| c != '\n' && font.has_glyph(c))
        .map(|c| red(font.outline_spans(c)).vertex_bound(segments))
        .sum()
}

#[cfg(test)]
pub(crate) mod testing {
    use cicada_core::config::ProjectConfig;
    use cicada_core::geometry::{Closed, Curve, Mesh, Watertight};
    use cicada_core::spatial::{Plane, Point};
    use glam::DVec3;

    use crate::output::text_outlines::{TextOutlinesIn, text_outlines};
    use crate::output::text_solids::{TextSolidsIn, text_solids};

    pub(crate) fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    pub(crate) fn outlines(text: &str, size: f64) -> Vec<Closed<Curve>> {
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

    pub(crate) fn solids(
        text: &str,
        size: f64,
        depth: f64,
        segments: i64,
    ) -> Vec<Watertight<Mesh>> {
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

    pub(crate) fn bbox(mesh: &Mesh) -> (DVec3, DVec3) {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        let (points, _) = mesh.positions().as_chunks::<3>();
        for &[x, y, z] in points {
            let v = DVec3::new(x, y, z);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (lo, hi)
    }

    pub(crate) fn polyline_vertices(curve: &Closed<Curve>) -> &[Point] {
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
    pub(crate) fn outline_area(curves: &[Closed<Curve>]) -> f64 {
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

    /// Number of cap triangles (z-facing) whose footprint contains (x, y):
    /// 2 inside the solid's footprint, 0 outside or in a counter.
    pub(crate) fn cap_crossings(mesh: &Mesh, x: f64, y: f64) -> usize {
        let pos = mesh.positions();
        let at = |i: u32| {
            let k = i as usize * 3;
            DVec3::new(pos[k], pos[k + 1], pos[k + 2])
        };
        let p = glam::DVec2::new(x, y);
        mesh.indices()
            .as_chunks::<3>()
            .0
            .iter()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // The allocation guard's bound, over every glyph EVERY bundled face
    // maps in the Basic Multilingual Plane (4,699 for DejaVu Sans Bold):
    // the vertices a glyph flattens to at 1, 2, 3, 8 and 64 chords a span
    // never exceed its span bound at that density — so `outline_vertex_bound`
    // is an upper bound on what `layout` allocates, for any text in any
    // bundled face, and a second face ships under the same assertion.
    // Lines are counted once: a line-only glyph's bound does not grow with
    // the density.
    #[test]
    fn span_bound_covers_every_glyph_of_every_bundled_face() {
        for name in bundled_font_names() {
            let font = bundled_font(name);
            let points = |c: char, segments: i64| -> usize {
                font.glyph(c, segments)
                    .unwrap()
                    .contours
                    .iter()
                    .map(Vec::len)
                    .sum()
            };
            let mut glyphs = 0usize;
            let mut heaviest = (' ', 0usize);
            let mut line_only = 0usize;
            for c in (0x20u32..=0xFFFF).filter_map(char::from_u32) {
                if !font.has_glyph(c) {
                    continue;
                }
                glyphs += 1;
                let spans = font.outline_spans(c).unwrap();
                for density in [1i64, 2, 3, 8, 64] {
                    let at = points(c, density);
                    let bound = spans.vertex_bound(u64::try_from(density).unwrap());
                    assert!(
                        u128::try_from(at).unwrap() <= bound,
                        "{name}: {c:?}: {at} vertices at {density} chords, bound {bound} from \
                         {spans:?}"
                    );
                }
                if spans.curves == 0 && spans.lines > 0 {
                    line_only += 1;
                    assert_eq!(
                        spans.vertex_bound(1_000_000),
                        spans.vertex_bound(1),
                        "{name}: {c:?} is line-only; its bound must not grow with the density"
                    );
                }
                if spans.curves > heaviest.1 {
                    heaviest = (c, spans.curves);
                }
            }
            assert!(glyphs > 4000, "{name} maps {glyphs} BMP characters");
            assert!(
                line_only > 10,
                "{name}: {line_only} line-only glyphs (A, H, L, …)"
            );
            assert!(
                heaviest.1 >= 100,
                "{name}: the heaviest glyph ({:?}) has {} bézier spans — a per-glyph constant \
                 would have to be that large for every character",
                heaviest.0,
                heaviest.1
            );
        }
        // The bound is the text's: a newline and an unmapped character
        // count nothing, the rest add up, lines are counted once and the
        // density multiplies the curves only.
        let font = bundled_font("DejaVu Sans Bold");
        let a = font.outline_spans('A').unwrap();
        let o = font.outline_spans('O').unwrap();
        assert_eq!(a.curves, 0, "A is straight lines: {a:?}");
        assert!(o.curves >= 8, "O is béziers: {o:?}");
        assert_eq!(
            outline_vertex_bound(font, "A\nO\u{1f41b}", 1),
            a.vertex_bound(1) + o.vertex_bound(1)
        );
        assert_eq!(
            outline_vertex_bound(font, "AO", 8),
            a.vertex_bound(1) + o.vertex_bound(8)
        );
        assert_eq!(
            outline_vertex_bound(font, "A", 2_000_000),
            outline_vertex_bound(font, "A", 1)
        );
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
}
