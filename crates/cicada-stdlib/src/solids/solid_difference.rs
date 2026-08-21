//! The `solid_difference` node (v0.1 item 3 WP-C).

use cicada_core::geometry::Solid;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`solid_difference`].
#[derive(Ports, Clone, Debug)]
pub struct SolidDifferenceIn {
    /// The solid to carve.
    pub solid: Solid,
    /// The solids to subtract, all in one pass (a through-hole's drill, a
    /// pocket's block).
    pub cutters: Vec<Solid>,
}

/// Solid Difference — subtract every cutter from a B-rep solid in one OCCT
/// pass, coplanar faces merged afterwards; the `mesh_difference` shape
/// (one solid, a list of cutters — lift with `each()` to carve per part),
/// but a `Solid` is always one body: a cut that splits the solid in two or
/// removes it entirely is red, never a compound or an empty solid.
///
/// # Returns
///
/// The carved solid — `solid` minus every cutter.
///
/// # Panics
///
/// Panics when the cut leaves anything but one solid (split or emptied), or
/// the kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=20.0)
/// thick = construct_domain(start=0.0, end=6.0)
/// plate = box(x=span, y=span, z=thick)
/// centre = construct_point(x=10.0, y=10.0, z=-1.0)
/// drill_frame = xy_plane(origin=centre)
/// drill = cylinder(plane=drill_frame, radius=3.0, height=8.0)
/// drills = duplicate(item=drill, count=1)
/// plate_with_hole = solid_difference(solid=plate, cutters=drills)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Solid Difference"
)]
#[must_use]
pub fn solid_difference(input: SolidDifferenceIn) -> Solid {
    red(cicada_geom::solid::difference_all(
        &input.solid,
        &input.cutters,
    ))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::solids::support::{
        brep_box, close_rel, config, plane_at, platform_golden, solid_hash, volume_of, with_kernel,
    };

    fn drill(x: f64, y: f64) -> Solid {
        cicada_geom::solid::cylinder(&plane_at(x, y, -1.0), 0.5, 6.0, config().tol()).unwrap()
    }

    #[test]
    fn solid_difference_table_cases() {
        let block = brep_box([0.0; 3], [4.0; 3]);
        // Two through-holes in one pass.
        let Some(drilled) = with_kernel(|| {
            solid_difference(SolidDifferenceIn {
                solid: block.clone(),
                cutters: vec![drill(1.0, 2.0), drill(3.0, 2.0)],
            })
        }) else {
            return;
        };
        assert!(close_rel(
            volume_of(&drilled),
            64.0 - 2.0 * PI * 0.25 * 4.0,
            1e-9
        ));
        // A corner notch (a box cutter): 64 − 1.
        let notched = solid_difference(SolidDifferenceIn {
            solid: block.clone(),
            cutters: vec![brep_box([3.0, 3.0, 3.0], [2.0; 3])],
        });
        assert!(close_rel(volume_of(&notched), 63.0, 1e-12));
        // A cutter that misses changes nothing but the bytes' history —
        // the volume is the block's.
        let missed = solid_difference(SolidDifferenceIn {
            solid: block.clone(),
            cutters: vec![brep_box([10.0; 3], [1.0; 3])],
        });
        assert!(close_rel(volume_of(&missed), 64.0, 1e-12));
        // No cutters: the block itself.
        assert_eq!(
            solid_difference(SolidDifferenceIn {
                solid: block.clone(),
                cutters: vec![],
            }),
            block
        );
    }

    #[test]
    fn solid_difference_that_splits_or_empties_is_red() {
        let Some(()) = with_kernel(|| {
            let block = brep_box([0.0; 3], [4.0; 3]);
            for cutter in [
                brep_box([-1.0, 1.5, -1.0], [6.0, 1.0, 6.0]), // a slab through the middle
                brep_box([-1.0; 3], [6.0; 3]),                // everything
            ] {
                let outcome = std::panic::catch_unwind(|| {
                    solid_difference(SolidDifferenceIn {
                        solid: block.clone(),
                        cutters: vec![cutter],
                    })
                });
                let payload = outcome.expect_err("not one solid");
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_default();
                assert!(message.contains("expected exactly one solid"), "{message}");
            }
        }) else {
            return;
        };
    }

    proptest::proptest! {
        // A box cutter at a corner region: |A \ B| = |A| − |A ∩ B| (kept
        // to cutters that leave one body: they enter from one corner).
        #[test]
        fn property_solid_difference_volume(
            sx in 0.5f64..1.9, sy in 0.5f64..1.9, sz in 0.5f64..1.9,
            ox in 1.0f64..1.5, oy in 1.0f64..1.5, oz in 1.0f64..1.5,
        ) {
            if cicada_geom::solid::kernel_available() {
                let extents = [2.0, 2.0, 2.0];
                let out = solid_difference(SolidDifferenceIn {
                    solid: brep_box([0.0; 3], extents),
                    cutters: vec![brep_box([ox, oy, oz], [sx, sy, sz])],
                });
                let overlap = crate::meshes::support::overlap_volume([0.0; 3], extents, [ox, oy, oz], [sx, sy, sz]);
                proptest::prop_assert!(close_rel(volume_of(&out), 8.0 - overlap, 1e-9));
            }
        }
    }

    #[test]
    fn solid_difference_determinism_golden_hash() {
        // A box with a box-shaped corner notch, exact coordinates. Blessed
        // via run-once on win-64 (2026-08-20).
        let Some(notched) = with_kernel(|| {
            solid_difference(SolidDifferenceIn {
                solid: brep_box([0.0; 3], [4.0, 3.0, 2.0]),
                cutters: vec![brep_box([3.0, 2.0, 1.0], [2.0; 3])],
            })
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&notched),
            platform_golden("a3c34d0efba738a1102e9b3603d518b5d60eece89cdc80541bfe2881d6a8100c")
        );
    }
}
