//! The `deconstruct_solid` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Solid};
use cicada_core::spatial::Point;
use cicada_geom::solid::Deflection;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`deconstruct_solid`].
#[derive(Ports, Clone, Debug)]
pub struct DeconstructSolidIn {
    /// The solid to take apart.
    pub solid: Solid,
    /// The chord deviation for edges the value model cannot hold exactly
    /// (arcs, ellipses, B-splines): they come back as polylines this close
    /// to the true edge, in document units.
    #[port(default = 0.01, dimension = length)]
    pub deflection: f64,
}

/// Outputs of [`deconstruct_solid`].
#[derive(Ports, Clone, Debug)]
pub struct DeconstructSolidOut {
    /// Every distinct edge: straight edges as `Line`s, full circles as
    /// `Circle`s, everything else a polyline at `deflection`; degenerate
    /// edges (a sphere's poles) are left out.
    pub edges: Vec<Curve>,
    /// Every distinct vertex.
    pub vertices: Vec<Point>,
    /// How many faces the solid has (there is no `Surface` kind yet, so the
    /// faces themselves are not output — this port's name leaves `faces`
    /// free for them).
    pub face_count: i64,
}

/// Deconstruct Solid — a B-rep solid's topology as values: its distinct
/// edges as curves, its distinct vertices as points, and its face count
/// (the faces themselves wait for the `Surface` kind).
///
/// # Panics
///
/// Panics when the deflection is below the kernel's floor (1e-7), or the
/// kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// edges, corners, faces = deconstruct_solid(solid=block)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Deconstruct Brep",
    uses_tolerance
)]
#[must_use]
pub fn deconstruct_solid(
    _config: &ProjectConfig,
    input: DeconstructSolidIn,
) -> DeconstructSolidOut {
    let deflection = red(Deflection::new(
        input.deflection,
        cicada_geom::solid::DISPLAY_ANGULAR_RAD,
    ));
    let (edges, vertices, faces) = red(cicada_geom::solid::edges_and_vertices(
        &input.solid,
        deflection,
    ));
    let face_count = i64::try_from(faces)
        .unwrap_or_else(|_| panic!("deconstruct_solid: {faces} faces do not fit an Integer"));
    DeconstructSolidOut {
        edges,
        vertices,
        face_count,
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Polyline;
    use cicada_core::spatial::Plane;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{brep_box, close_rel, config, with_kernel};

    #[test]
    fn deconstruct_solid_table_cases() {
        // A box: 12 line edges, 8 corners, 6 faces; the edge lengths sum to
        // 4 (a + b + c).
        let Some(out) = with_kernel(|| {
            deconstruct_solid(
                &config(),
                DeconstructSolidIn {
                    solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                    deflection: 0.01,
                },
            )
        }) else {
            return;
        };
        assert_eq!(out.face_count, 6);
        assert_eq!(out.vertices.len(), 8);
        assert_eq!(out.edges.len(), 12);
        assert!(out.edges.iter().all(|e| matches!(e, Curve::Line(_))));
        let total: f64 = out
            .edges
            .iter()
            .map(|e| cicada_geom::curve::length(e, config().tol()).unwrap())
            .sum();
        assert!(close_rel(total, 24.0, 1e-12));
        // A cylinder: two circular rims (exact), one seam line, 3 faces.
        let peg =
            cicada_geom::solid::cylinder(&Plane::world_xy(), 1.5, 4.0, config().tol()).unwrap();
        let out = deconstruct_solid(
            &config(),
            DeconstructSolidIn {
                solid: peg,
                deflection: 0.01,
            },
        );
        assert_eq!(out.face_count, 3);
        let circles = out
            .edges
            .iter()
            .filter(|e| matches!(e, Curve::Circle(_)))
            .count();
        assert_eq!(circles, 2, "{:?}", out.edges);
        assert!(
            out.edges.iter().any(|e| matches!(e, Curve::Line(_))),
            "the seam"
        );
        // A sphere: one face, two poles, one seam (a semicircle → polyline).
        let ball = cicada_geom::solid::sphere(&Plane::world_xy(), 1.0, config().tol()).unwrap();
        let out = deconstruct_solid(
            &config(),
            DeconstructSolidIn {
                solid: ball,
                deflection: 0.001,
            },
        );
        assert_eq!(out.face_count, 1);
        assert_eq!(out.vertices.len(), 2);
        assert_eq!(out.edges.len(), 1);
        let Curve::Polyline(Polyline { vertices, closed }) = &out.edges[0] else {
            panic!("the seam is a discretized semicircle: {:?}", out.edges);
        };
        assert!(!closed);
        assert!(vertices.len() > 10);
        assert!(
            vertices.iter().all(|v| tol::close(v.0.length(), 1.0, 2e-3)),
            "on the sphere"
        );
    }

    #[test]
    #[should_panic(expected = "linear_deflection")]
    fn deconstruct_solid_below_the_kernel_floor_is_red() {
        let _ = deconstruct_solid(
            &config(),
            DeconstructSolidIn {
                solid: Solid::from_canonical_bytes(
                    cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec(),
                )
                .unwrap(),
                deflection: 1e-9,
            },
        );
    }

    proptest::proptest! {
        // Euler for a box, whatever its size: V − E + F = 2, and the
        // vertices are the eight corners.
        #[test]
        fn property_deconstruct_box_euler(
            sx in 0.1f64..20.0, sy in 0.1f64..20.0, sz in 0.1f64..20.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = deconstruct_solid(
                    &config(),
                    DeconstructSolidIn {
                        solid: brep_box([0.0; 3], [sx, sy, sz]),
                        deflection: 0.01,
                    },
                );
                let (v, e) = (
                    i64::try_from(out.vertices.len()).unwrap(),
                    i64::try_from(out.edges.len()).unwrap(),
                );
                proptest::prop_assert_eq!(v - e + out.face_count, 2);
                let on_corners = out.vertices.iter().all(|p| {
                    (tol::close(p.0.x, 0.0, 1e-9) || tol::close(p.0.x, sx, 1e-9))
                        && (tol::close(p.0.y, 0.0, 1e-9) || tol::close(p.0.y, sy, 1e-9))
                        && (tol::close(p.0.z, 0.0, 1e-9) || tol::close(p.0.z, sz, 1e-9))
                });
                proptest::prop_assert!(on_corners);
            }
        }
    }

    #[test]
    fn deconstruct_solid_determinism_golden_hash() {
        // The 1 × 2 × 3 box's vertices, as the list value: exact corners in
        // the kernel's (deterministic) order.
        let Some(out) = with_kernel(|| {
            deconstruct_solid(
                &config(),
                DeconstructSolidIn {
                    solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                    deflection: 0.01,
                },
            )
        }) else {
            return;
        };
        let slots: Vec<_> = out
            .vertices
            .iter()
            .map(|p| Some(HashedValue::new(ValueData::Point(*p)).unwrap()))
            .collect();
        let list = HashedValue::new(ValueData::List(cicada_core::value::List {
            axis: None,
            slots,
        }))
        .unwrap();
        assert_eq!(
            list.hash().to_hex(),
            crate::solids::support::platform_golden(
                "de1d9bf3254d66922269cf516ac2826506ab6df4279eda0ca83d80987bdec207"
            )
        );
    }
}
