//! The `area` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Circle, Closed, Curve, Rectangle};
use cicada_core::spatial::Point;
use cicada_geom::GeomError;
use cicada_geom::frame::{orthonormal, polygon_frame};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`area`].
#[derive(Ports, Clone, Debug)]
pub struct AreaIn {
    /// The closed planar curve bounding the region.
    pub curve: Closed<Curve>,
}

/// Outputs of [`area`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct AreaOut {
    /// The enclosed area — always positive, whichever way the curve winds.
    pub area: f64,
    /// The area centroid (first moment over area), in the curve's plane.
    pub centroid: Point,
}

/// Area — the enclosed area and the area centroid of a closed planar
/// curve (the spike form of docs/08 §7 Area: closed curves only; surfaces
/// arrive with v0.1). Polylines by the shoelace formula in their own plane
/// (Newell normal) — the centroid is the AREA centroid, not the vertex
/// mean (the wall scales its Voronoi cells about it); circles and
/// rectangles analytically. The area is orientation-independent (always
/// positive). Self-intersecting polylines are not detected: the shoelace
/// result is what it is (lobes of opposite winding cancel).
///
/// # Panics
///
/// Panics when the curve is degenerate at tolerance (fewer than 3
/// distinct vertices, a zero radius or empty extent, or an enclosed area
/// within tolerance² of zero — collinear vertices), non-planar within
/// tolerance, or its frame is degenerate.
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// measure, centroid = area(curve=ring)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Area",
    uses_tolerance
)]
#[must_use]
pub fn area(config: &ProjectConfig, input: AreaIn) -> AreaOut {
    let tol = config.tol();
    let (area, centroid) = match &input.curve.0 {
        Curve::Circle(Circle { plane, radius }) => {
            assert!(
                *radius > tol,
                "degenerate curve: circle radius {radius} is not above tolerance {tol}"
            );
            let frame = red(orthonormal(plane, tol));
            (std::f64::consts::PI * radius * radius, frame.origin)
        }
        Curve::Rectangle(Rectangle { plane, x, y }) => {
            for (name, domain) in [("x", x), ("y", y)] {
                assert!(
                    !cicada_geom::tol::close(domain.start, domain.end, tol),
                    "degenerate curve: rectangle {name} extent {}..{} is empty at tolerance {tol}",
                    domain.start,
                    domain.end
                );
            }
            let frame = red(orthonormal(plane, tol));
            (
                (x.end - x.start).abs() * (y.end - y.start).abs(),
                frame.point_at(f64::midpoint(x.start, x.end), f64::midpoint(y.start, y.end)),
            )
        }
        // Vertex chains (and anything else the refinement lets through):
        // tolerance-deduped corners, refused if open or under 3 distinct.
        // `segments` only matters for curved variants, handled above.
        curve => {
            let corners = red(cicada_geom::curve::tessellate_closed(curve, 3, tol));
            red(polygon_area_centroid(&corners, tol))
        }
    };
    AreaOut { area, centroid }
}

/// Shoelace area and area centroid of a closed polygon loop (no duplicate
/// closing vertex) in its own plane: the loop's Newell frame (origin at
/// the vertex mean keeps the 2-D coordinates small), every vertex checked
/// onto that plane, then `A = ½ Σ (xᵢ yᵢ₊₁ − xᵢ₊₁ yᵢ)` and
/// `C = Σ (vᵢ + vᵢ₊₁)(xᵢ yᵢ₊₁ − xᵢ₊₁ yᵢ) / 6A` — the signs cancel, so
/// the centroid is orientation-safe and the area is returned unsigned.
fn polygon_area_centroid(loop_points: &[Point], tol: f64) -> Result<(f64, Point), GeomError> {
    // A zero Newell normal IS zero area (collinear or coincident loop):
    // report it as the area failure it is, not as a frame problem.
    let frame = polygon_frame(loop_points, tol).map_err(|_| GeomError::DegenerateCurve {
        reason: format!(
            "enclosed area is within tolerance of zero (tolerance {tol}; collinear or \
             coincident vertices?)"
        ),
    })?;
    let mut flat = Vec::with_capacity(loop_points.len());
    for (vertex, point) in loop_points.iter().enumerate() {
        let local = frame.coordinates(*point);
        if !cicada_geom::tol::near_zero(local.z, tol) {
            return Err(GeomError::NotPlanar {
                vertex,
                distance: local.z,
            });
        }
        flat.push((local.x, local.y));
    }
    let mut twice_area = 0.0;
    let mut moment_x = 0.0;
    let mut moment_y = 0.0;
    for (i, &(ax, ay)) in flat.iter().enumerate() {
        let (bx, by) = flat[(i + 1) % flat.len()];
        let cross = ax * by - bx * ay;
        twice_area += cross;
        moment_x += (ax + bx) * cross;
        moment_y += (ay + by) * cross;
    }
    let signed_area = twice_area / 2.0;
    // Area comparisons use tol² (the crate's convention, see `ear_clip`).
    if cicada_geom::tol::near_zero(signed_area, tol * tol) {
        return Err(GeomError::DegenerateCurve {
            reason: format!(
                "enclosed area {} is within tolerance of zero (tolerance {tol}; area \
                 tolerance {})",
                signed_area.abs(),
                tol * tol
            ),
        });
    }
    let centroid = frame.point_at(moment_x / (3.0 * twice_area), moment_y / (3.0 * twice_area));
    Ok((signed_area.abs(), centroid))
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Polyline;
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Vector};
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::curves::circle::{CircleIn, circle};
    use crate::curves::rectangle::{RectangleIn, rectangle};
    use crate::solids::support::config;

    /// A closed polyline through `(x, y)` corners at height `z`.
    fn ring(corners: &[(f64, f64)], z: f64) -> Closed<Curve> {
        loop3(&corners.iter().map(|&(x, y)| (x, y, z)).collect::<Vec<_>>())
    }

    /// A closed polyline through `(x, y, z)` vertices.
    fn loop3(vertices: &[(f64, f64, f64)]) -> Closed<Curve> {
        Closed(Curve::Polyline(Polyline {
            vertices: vertices
                .iter()
                .map(|&(x, y, z)| Point::new(x, y, z))
                .collect(),
            closed: true,
        }))
    }

    /// Geometry asserts are tolerance-aware (doc 14): relative for the
    /// area, Euclidean for the centroid.
    fn assert_area(out: &AreaOut, area: f64, centroid: Point, what: &str) {
        assert!(
            (out.area - area).abs() <= 1e-9 * area.max(1.0),
            "{what}: area {} want {area}",
            out.area
        );
        assert!(
            out.centroid.0.distance(centroid.0) <= 1e-9,
            "{what}: centroid {:?} want {:?}",
            out.centroid.0,
            centroid.0
        );
    }

    /// Three unit squares: the AREA centroid (5/6, 5/6) is not the vertex
    /// mean (1, 1) — the case the node exists for.
    const L_SHAPE: [(f64, f64); 6] = [
        (0.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (1.0, 2.0),
        (0.0, 2.0),
    ];

    #[test]
    fn area_polygon_table_cases() {
        let unit_square = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mut clockwise = unit_square;
        clockwise.reverse();
        let centre = |x, y, z| Point::new(x, y, z);
        let cases: Vec<(&str, Closed<Curve>, f64, Point)> = vec![
            (
                "unit square",
                ring(&unit_square, 0.0),
                1.0,
                centre(0.5, 0.5, 0.0),
            ),
            // The area stays positive whichever way the loop winds.
            (
                "unit square, clockwise",
                ring(&clockwise, 0.0),
                1.0,
                centre(0.5, 0.5, 0.0),
            ),
            // The centroid lies in the curve's plane, at its z.
            (
                "unit square at z = 4",
                ring(&unit_square, 4.0),
                1.0,
                centre(0.5, 0.5, 4.0),
            ),
            // Consecutive coincident vertices dedupe at tolerance.
            (
                "unit square with a duplicated vertex",
                ring(
                    &[(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                    0.0,
                ),
                1.0,
                centre(0.5, 0.5, 0.0),
            ),
            (
                "L-shape",
                ring(&L_SHAPE, 0.0),
                3.0,
                centre(5.0 / 6.0, 5.0 / 6.0, 0.0),
            ),
            (
                "3-4-5 triangle",
                ring(&[(0.0, 0.0), (4.0, 0.0), (0.0, 3.0)], 0.0),
                6.0,
                centre(4.0 / 3.0, 1.0, 0.0),
            ),
            // A 4×4 square with a triangular notch (area 4) cut into its top:
            // 16 − 4 = 12; centroid by subtraction = (2, 14/9).
            (
                "non-convex arrow",
                ring(
                    &[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 2.0), (0.0, 4.0)],
                    0.0,
                ),
                12.0,
                centre(2.0, 14.0 / 9.0, 0.0),
            ),
        ];
        for (what, curve, want_area, want_centroid) in cases {
            let out = area(&config(), AreaIn { curve });
            assert_area(&out, want_area, want_centroid, what);
        }
    }

    #[test]
    fn area_out_of_plane_polygon_table_cases() {
        let cases: Vec<(&str, Closed<Curve>, f64, Point)> = vec![
            (
                "unit square in the XZ plane",
                loop3(&[
                    (0.0, 0.0, 0.0),
                    (1.0, 0.0, 0.0),
                    (1.0, 0.0, 1.0),
                    (0.0, 0.0, 1.0),
                ]),
                1.0,
                Point::new(0.5, 0.0, 0.5),
            ),
            // Edges (2,0,0) and (0,2,2) are perpendicular: a 2 × 2√2
            // rectangle tilted 45° out of XY.
            (
                "tilted rectangle",
                loop3(&[
                    (0.0, 0.0, 0.0),
                    (2.0, 0.0, 0.0),
                    (2.0, 2.0, 2.0),
                    (0.0, 2.0, 2.0),
                ]),
                4.0 * 2.0_f64.sqrt(),
                Point::new(1.0, 1.0, 1.0),
            ),
            // The L-shape turned a quarter turn CCW about the origin
            // ((x, y) → (−y, x)) and moved by (5, 7, 2): same area, the
            // centroid follows the rigid motion.
            (
                "L-shape, rotated + translated",
                loop3(
                    &L_SHAPE
                        .iter()
                        .map(|&(x, y)| (5.0 - y, 7.0 + x, 2.0))
                        .collect::<Vec<_>>(),
                ),
                3.0,
                Point::new(5.0 - 5.0 / 6.0, 7.0 + 5.0 / 6.0, 2.0),
            ),
        ];
        for (what, curve, want_area, want_centroid) in cases {
            let out = area(&config(), AreaIn { curve });
            assert_area(&out, want_area, want_centroid, what);
        }
    }

    #[test]
    fn area_analytic_table_cases() {
        let cases: Vec<(&str, Closed<Curve>, f64, Point)> = vec![
            (
                "circle, analytic",
                circle(
                    &config(),
                    CircleIn {
                        plane: Plane {
                            origin: Point::new(1.0, 2.0, 3.0),
                            x: Vector::new(3.0, 0.0, 0.0),
                            y: Vector::new(1.0, 2.0, 0.0),
                        },
                        radius: 2.0,
                    },
                ),
                4.0 * std::f64::consts::PI,
                Point::new(1.0, 2.0, 3.0),
            ),
            (
                "rectangle node output",
                rectangle(
                    &config(),
                    RectangleIn {
                        plane: Plane::world_xy(),
                        x: Domain::new(0.0, 3.0),
                        y: Domain::new(-1.0, 2.0),
                    },
                ),
                9.0,
                Point::new(1.5, 0.5, 0.0),
            ),
            (
                "rectangle, decreasing extent in a lifted frame",
                Closed(Curve::Rectangle(Rectangle {
                    plane: Plane {
                        origin: Point::new(0.0, 0.0, 5.0),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(3.0, 0.0),
                    y: Domain::new(0.0, 2.0),
                })),
                6.0,
                Point::new(1.5, 1.0, 5.0),
            ),
        ];
        for (what, curve, want_area, want_centroid) in cases {
            let out = area(&config(), AreaIn { curve });
            assert_area(&out, want_area, want_centroid, what);
        }
    }

    #[test]
    #[should_panic(expected = "area")]
    fn area_collinear_polyline_is_red() {
        let _ = area(
            &config(),
            AreaIn {
                curve: ring(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "distinct vertices")]
    fn area_two_vertex_loop_is_red() {
        let _ = area(
            &config(),
            AreaIn {
                curve: ring(&[(0.0, 0.0), (1.0, 0.0)], 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "not planar")]
    fn area_non_planar_polyline_is_red() {
        let _ = area(
            &config(),
            AreaIn {
                curve: loop3(&[
                    (0.0, 0.0, 0.0),
                    (1.0, 0.0, 0.0),
                    (1.0, 1.0, 1.0),
                    (0.0, 1.0, 0.0),
                ]),
            },
        );
    }

    #[test]
    #[should_panic(expected = "radius")]
    fn area_zero_radius_circle_is_red() {
        let _ = area(
            &config(),
            AreaIn {
                curve: Closed(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 0.0,
                })),
            },
        );
    }

    #[test]
    #[should_panic(expected = "extent")]
    fn area_empty_rectangle_is_red() {
        let _ = area(
            &config(),
            AreaIn {
                curve: Closed(Curve::Rectangle(Rectangle {
                    plane: Plane::world_xy(),
                    x: Domain::new(1.0, 1.0),
                    y: Domain::new(0.0, 2.0),
                })),
            },
        );
    }

    proptest::proptest! {
        // Scaling a polygon by `s` about a center and translating it
        // multiplies the area by s² and carries the centroid along the
        // same similarity; reversing the loop changes nothing. Polygons
        // are star-shaped about the origin (random radii at equal
        // angles), so they are simple by construction.
        #[test]
        fn property_area_similarity_and_orientation(
            radii in proptest::collection::vec(0.5..5.0_f64, 3..12),
            s in 0.1..10.0_f64,
            cx in -10.0..10.0_f64, cy in -10.0..10.0_f64,
            tx in -100.0..100.0_f64, ty in -100.0..100.0_f64, tz in -100.0..100.0_f64,
        ) {
            #[allow(clippy::cast_precision_loss)]
            let base: Vec<(f64, f64)> = radii
                .iter()
                .enumerate()
                .map(|(i, &r)| {
                    let angle = std::f64::consts::TAU * i as f64 / radii.len() as f64;
                    (r * angle.cos(), r * angle.sin())
                })
                .collect();
            let similar: Vec<(f64, f64)> = base
                .iter()
                .map(|&(x, y)| (cx + s * (x - cx) + tx, cy + s * (y - cy) + ty))
                .collect();
            let mut reversed = similar.clone();
            reversed.reverse();

            let a = area(&config(), AreaIn { curve: ring(&base, 0.0) });
            let b = area(&config(), AreaIn { curve: ring(&similar, tz) });
            let c = area(&config(), AreaIn { curve: ring(&reversed, tz) });

            let want_area = s * s * a.area;
            proptest::prop_assert!(
                (b.area - want_area).abs() <= 1e-9 * want_area.max(1.0),
                "area {} want {}", b.area, want_area
            );
            let want_centroid = Point::new(
                cx + s * (a.centroid.0.x - cx) + tx,
                cy + s * (a.centroid.0.y - cy) + ty,
                tz,
            );
            let scale = want_centroid.0.length().max(1.0);
            proptest::prop_assert!(
                b.centroid.0.distance(want_centroid.0) <= 1e-9 * scale,
                "centroid {:?} want {:?}", b.centroid.0, want_centroid.0
            );
            proptest::prop_assert!((c.area - b.area).abs() <= 1e-12 * b.area.max(1.0));
            proptest::prop_assert!(c.centroid.0.distance(b.centroid.0) <= 1e-9 * scale);
        }
    }

    // The catalog row docs/08 §7 promises, through the real macro: the
    // refined input, both outputs, the category, and the tolerance flag.
    #[test]
    fn area_spec_roundtrips_signature_and_contract() {
        let spec = crate::registry()
            .iter()
            .find(|s| s.name == "area")
            .expect("area registered");
        assert_eq!(
            spec.signature(),
            "area(curve: Closed<Curve>) → (area: Number, centroid: Point)"
        );
        assert_eq!(spec.category, "Surface & solid");
        assert!(spec.uses_tolerance && spec.pure);
        let panics = spec.panics.expect("area has a Red-when contract");
        assert!(panics.contains("non-planar"), "{panics}");
    }

    #[test]
    fn area_determinism_golden_hash() {
        // Arithmetic-only inputs: integer corners whose Newell frame has an
        // axis-aligned normal and a 3-4-5 first-vertex offset (no libm —
        // sqrt is correctly rounded everywhere, trig is not).
        let out = area(
            &config(),
            AreaIn {
                curve: ring(&[(0.0, 0.0), (3.0, 0.0), (3.0, 4.0), (0.0, 4.0)], 0.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Number(out.area))
                .unwrap()
                .hash()
                .to_hex(),
            "f45338c7b6a6f9435f105c5e0daf777100f3f5aa061eb711ad82969f63cc7c83"
        );
        assert_eq!(
            HashedValue::new(ValueData::Point(out.centroid))
                .unwrap()
                .hash()
                .to_hex(),
            "f98ce4d1b7090fa548725bd96986cc58273c78c79ada3ea016ca028eb1e4fc1a"
        );
        // The L-shape: the area centroid the wall scales its cells about.
        let out = area(
            &config(),
            AreaIn {
                curve: ring(&L_SHAPE, 0.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Number(out.area))
                .unwrap()
                .hash()
                .to_hex(),
            "44cdfdc88ab8d92ffed745510e256977ab0cc0cf7b6511f051d311ba4f2adb7e"
        );
        assert_eq!(
            HashedValue::new(ValueData::Point(out.centroid))
                .unwrap()
                .hash()
                .to_hex(),
            "25f11b60153b5835a37146f944573758fbfbc2893d9eec4c8d0fcee84ae824c5"
        );
    }
}
