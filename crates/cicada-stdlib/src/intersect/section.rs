//! The `section` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_core::spatial::Plane;
use cicada_geom::solid::Deflection;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`section`].
#[derive(Ports, Clone, Debug)]
pub struct SectionIn {
    /// The solid to cut.
    pub solid: Solid,
    /// The cutting plane.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The chord deviation for curves the value model cannot hold exactly
    /// (ellipses, B-splines): they come back as polylines this close to the
    /// true curve, in document units.
    #[port(default = 0.01, dimension = length)]
    pub deflection: f64,
}

/// Section — the planar section of a B-rep solid: one closed curve per
/// loop the plane cuts (a through-hole makes two). A loop that is one full
/// circle comes back as an exact `Circle`; every other loop is a closed
/// polyline — exact corners for planar faces, `deflection`-close points
/// for curved ones. A tangent contact — the plane touching the solid
/// along a line or curve without entering it there (tangent to a cylinder
/// along a generatrix, through one edge of a box, grazing a bore's wall)
/// — bounds no region and contributes no loop. An empty list when the
/// plane misses the solid, or only touches it.
///
/// # Returns
///
/// The section loops, each a closed curve.
///
/// # Panics
///
/// Panics when the plane is degenerate, the deflection is below the
/// kernel's floor (1e-7), when the kernel leaves a loop open (an open
/// chain with the solid on one side of it — a kernel failure on this
/// solid, never returned as a loop), or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=20.0)
/// thick = construct_domain(start=0.0, end=6.0)
/// plate = box(x=span, y=span, z=thick)
/// mid = construct_point(z=3.0)
/// cut_plane = xy_plane(origin=mid)
/// outline = section(solid=plate, plane=cut_plane)
/// ```
// `version = 2`: a tangent contact yields no loop where it was a (false)
// red — a behavior change on inputs the memo never held, bumped all the
// same (the rule is the rule; a section recomputes in milliseconds).
#[node(
    category = "Intersect & regions",
    tier = "1",
    version = 2,
    gh = "Brep | Plane",
    uses_tolerance
)]
#[must_use]
pub fn section(config: &ProjectConfig, input: SectionIn) -> Vec<Closed<Curve>> {
    // The angular deviation is the display's: fine enough for a loop that
    // will be re-extruded, coarse enough not to explode a polyline.
    let deflection = red(Deflection::new(
        input.deflection,
        cicada_geom::solid::DISPLAY_ANGULAR_RAD,
    ));
    let loops = red(cicada_geom::solid::section(
        &input.solid,
        &input.plane,
        config.tol(),
        deflection,
    ));
    loops
        .into_iter()
        .map(|curve| {
            // The seam drops tangent contacts and refuses an open chain
            // with the solid on one side; this restates, at the node, the
            // invariant the `Closed` wrapper promises.
            assert!(
                curve.is_closed(),
                "section: the kernel returned an open curve ({}) as a loop — a loop that did \
                 not close is a kernel failure on this solid",
                curve.variant_name()
            );
            Closed(curve)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Polyline};
    use cicada_core::spatial::{Point, Vector};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{brep_box, close_rel, config, plane_at, with_kernel};

    fn drilled_plate() -> Solid {
        let plate = brep_box([0.0; 3], [20.0, 20.0, 6.0]);
        let drill =
            cicada_geom::solid::cylinder(&plane_at(10.0, 10.0, -1.0), 3.0, 8.0, config().tol())
                .unwrap();
        cicada_geom::solid::difference_all(&plate, &[drill]).unwrap()
    }

    #[test]
    fn section_table_cases() {
        // A drilled plate cut through the middle: the outline and the hole.
        let Some(loops) = with_kernel(|| {
            section(
                &config(),
                SectionIn {
                    solid: drilled_plate(),
                    plane: plane_at(0.0, 0.0, 3.0),
                    deflection: 0.01,
                },
            )
        }) else {
            return;
        };
        assert_eq!(loops.len(), 2, "{loops:?}");
        let outline = loops
            .iter()
            .find_map(|c| match &c.0 {
                Curve::Polyline(p) => Some(p),
                _ => None,
            })
            .expect("the outline is a polyline");
        assert!(outline.closed);
        assert_eq!(outline.vertices.len(), 4);
        assert!(
            outline
                .vertices
                .iter()
                .all(|v| tol::close(v.0.z, 3.0, 1e-9))
        );
        let hole = loops
            .iter()
            .find_map(|c| match &c.0 {
                Curve::Circle(circle) => Some(circle),
                _ => None,
            })
            .expect("the hole is an exact circle");
        assert!(tol::close(hole.radius, 3.0, 1e-9));
        assert!(tol::coincident(
            hole.plane.origin,
            Point::new(10.0, 10.0, 3.0),
            1e-9
        ));
        // The hole's area is exact: the section re-feeds `area` and `extrude`.
        let measured = crate::solids::area::area(
            &config(),
            crate::solids::area::AreaIn {
                curve: Closed(Curve::Circle(*hole)),
            },
        );
        assert!(close_rel(measured.area, std::f64::consts::PI * 9.0, 1e-12));
        // A plane that misses: nothing.
        let none = section(
            &config(),
            SectionIn {
                solid: drilled_plate(),
                plane: plane_at(0.0, 0.0, 50.0),
                deflection: 0.01,
            },
        );
        assert!(none.is_empty());
        // A plane that only TOUCHES — tangent to a cylinder along one
        // generatrix (the review's false red: "an open loop (Line)") —
        // bounds no region: nothing, like a miss.
        let peg =
            cicada_geom::solid::cylinder(&Plane::world_xy(), 1.0, 3.0, config().tol()).unwrap();
        let tangent = section(
            &config(),
            SectionIn {
                solid: peg,
                plane: Plane {
                    origin: Point::new(1.0, 0.0, 0.0),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                deflection: 0.01,
            },
        );
        assert!(tangent.is_empty(), "{tangent:?}");
        // A plane grazing the drilled plate's hole from inside the material
        // (y = 13, the hole's wall at its widest) keeps the loop it does
        // cut — the plate's outline — and drops the graze.
        let grazed = section(
            &config(),
            SectionIn {
                solid: drilled_plate(),
                plane: Plane {
                    origin: Point::new(0.0, 13.0, 0.0),
                    x: Vector::new(1.0, 0.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                deflection: 0.01,
            },
        );
        assert_eq!(grazed.len(), 1, "{grazed:?}");
        assert!(grazed[0].0.is_closed());
        // A vertical cut through a sphere: one circle of the right radius.
        let ball = cicada_geom::solid::sphere(&Plane::world_xy(), 2.0, config().tol()).unwrap();
        let cut = section(
            &config(),
            SectionIn {
                solid: ball,
                plane: plane_at(0.0, 0.0, 1.0),
                deflection: 0.01,
            },
        );
        assert_eq!(cut.len(), 1);
        let Curve::Circle(Circle { radius, .. }) = &cut[0].0 else {
            panic!("a sphere's section is a circle: {cut:?}");
        };
        assert!(tol::close(*radius, 3.0f64.sqrt(), 1e-9), "{radius}");
    }

    #[test]
    #[should_panic(expected = "linear_deflection")]
    fn section_below_the_kernel_floor_is_red() {
        let _ = section(
            &config(),
            SectionIn {
                solid: Solid::from_canonical_bytes(
                    cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec(),
                )
                .unwrap(),
                plane: Plane::world_xy(),
                deflection: 1e-9,
            },
        );
    }

    proptest::proptest! {
        // Any horizontal cut through a box is its footprint: four corners
        // at the cut height.
        #[test]
        fn property_section_of_a_box_is_its_footprint(
            sx in 0.5f64..10.0, sy in 0.5f64..10.0, sz in 0.5f64..10.0,
            t in 0.05f64..0.95,
        ) {
            if cicada_geom::solid::kernel_available() {
                let z = sz * t;
                let out = section(
                    &config(),
                    SectionIn {
                        solid: brep_box([0.0; 3], [sx, sy, sz]),
                        plane: plane_at(0.0, 0.0, z),
                        deflection: 0.01,
                    },
                );
                proptest::prop_assert_eq!(out.len(), 1);
                let Curve::Polyline(Polyline { vertices, closed }) = &out[0].0 else {
                    return Err(proptest::test_runner::TestCaseError::fail("not a polyline"));
                };
                proptest::prop_assert!(*closed);
                proptest::prop_assert_eq!(vertices.len(), 4);
                proptest::prop_assert!(vertices.iter().all(|v| tol::close(v.0.z, z, 1e-9)));
            }
        }
    }

    #[test]
    fn section_determinism_golden_hash() {
        // The 1 × 2 × 3 box cut at z = 1: four exact corners, a
        // transcendental-free closed polyline whose hash is pure arithmetic.
        let Some(loops) = with_kernel(|| {
            section(
                &config(),
                SectionIn {
                    solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                    plane: plane_at(0.0, 0.0, 1.0),
                    deflection: 0.01,
                },
            )
        }) else {
            return;
        };
        assert_eq!(loops.len(), 1);
        let sealed = HashedValue::new(ValueData::Curve(loops[0].0.clone())).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            crate::solids::support::platform_golden(
                "dc1df3d5b0968ab95b90eb5bfe8e2edb51a149d7fb0f65ff9fae94defca556a6"
            )
        );
    }
}
