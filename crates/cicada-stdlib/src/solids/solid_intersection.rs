//! The `solid_intersection` node (v0.1 item 3 WP-C).

use cicada_core::geometry::Solid;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`solid_intersection`].
#[derive(Ports, Clone, Debug)]
pub struct SolidIntersectionIn {
    /// The first solid.
    pub a: Solid,
    /// The second solid.
    pub b: Solid,
}

/// Solid Intersection — the common volume of two B-rep solids (OCCT
/// common), coplanar faces merged afterwards; the `mesh_intersection`
/// shape, but a `Solid` is always one body: solids that do not overlap, or
/// overlap in several pieces, are red — there is no empty solid.
///
/// # Returns
///
/// The common solid.
///
/// # Panics
///
/// Panics when the solids do not overlap in exactly one body, or the kernel
/// refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// ball = sphere(radius=1.5)
/// core = solid_intersection(a=block, b=ball)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Solid Intersection"
)]
#[must_use]
pub fn solid_intersection(input: SolidIntersectionIn) -> Solid {
    red(cicada_geom::solid::intersection(&input.a, &input.b))
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{
        bounds_of, brep_box, close_rel, platform_golden, solid_hash, volume_of, with_kernel,
    };

    #[test]
    fn solid_intersection_table_cases() {
        let Some(common) = with_kernel(|| {
            solid_intersection(SolidIntersectionIn {
                a: brep_box([0.0; 3], [2.0; 3]),
                b: brep_box([1.0; 3], [2.0; 3]),
            })
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&common), 1.0, 1e-12));
        let (min, max) = bounds_of(&common);
        assert!(tol::coincident(min, Point::new(1.0, 1.0, 1.0), 1e-9));
        assert!(tol::coincident(max, Point::new(2.0, 2.0, 2.0), 1e-9));
        // Commutative.
        let swapped = solid_intersection(SolidIntersectionIn {
            a: brep_box([1.0; 3], [2.0; 3]),
            b: brep_box([0.0; 3], [2.0; 3]),
        });
        assert!(close_rel(volume_of(&swapped), 1.0, 1e-12));
        // A solid with itself is itself (in volume).
        let same = solid_intersection(SolidIntersectionIn {
            a: brep_box([0.0; 3], [2.0; 3]),
            b: brep_box([0.0; 3], [2.0; 3]),
        });
        assert!(close_rel(volume_of(&same), 8.0, 1e-12));
    }

    #[test]
    fn solid_intersection_of_disjoint_solids_is_red() {
        let Some(()) = with_kernel(|| {
            let outcome = std::panic::catch_unwind(|| {
                solid_intersection(SolidIntersectionIn {
                    a: brep_box([0.0; 3], [1.0; 3]),
                    b: brep_box([5.0; 3], [1.0; 3]),
                })
            });
            let payload = outcome.expect_err("no common volume is not a solid");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(
                message.contains(
                    "intersection left no solid — a Solid is one body, and nothing remains"
                ),
                "{message}"
            );
            assert!(!message.contains("cicada_"), "{message}");
        }) else {
            return;
        };
    }

    proptest::proptest! {
        // Two overlapping world-aligned boxes: |A ∩ B| is the product of
        // the per-axis overlaps.
        #[test]
        fn property_solid_intersection_volume(
            ox in 0.1f64..1.9, oy in 0.1f64..1.9, oz in 0.1f64..1.9,
            sx in 0.5f64..3.0, sy in 0.5f64..3.0, sz in 0.5f64..3.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let extents = [2.0, 2.0, 2.0];
                let out = solid_intersection(SolidIntersectionIn {
                    a: brep_box([0.0; 3], extents),
                    b: brep_box([ox, oy, oz], [sx, sy, sz]),
                });
                let want = crate::meshes::support::overlap_volume([0.0; 3], extents, [ox, oy, oz], [sx, sy, sz]);
                proptest::prop_assert!(close_rel(volume_of(&out), want, 1e-9));
            }
        }
    }

    #[test]
    fn solid_intersection_determinism_golden_hash() {
        // Two overlapping boxes, exact coordinates. Blessed via run-once on
        // win-64 (2026-08-20).
        let Some(common) = with_kernel(|| {
            solid_intersection(SolidIntersectionIn {
                a: brep_box([0.0; 3], [4.0, 3.0, 2.0]),
                b: brep_box([1.0, 1.0, 1.0], [4.0, 3.0, 2.0]),
            })
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&common),
            platform_golden("6be9f8f2e48b431427ea58264e507a7ee9985cab73ce37aa1cde12b656c760dc")
        );
    }
}
