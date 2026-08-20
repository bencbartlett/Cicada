//! The bundled-font table shared by the text nodes, plus their test
//! helpers.

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
