//! The `solid_union` node (v0.1 item 3 WP-C).

use cicada_core::geometry::Solid;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`solid_union`].
#[derive(Ports, Clone, Debug)]
pub struct SolidUnionIn {
    /// The solids to union — they must overlap or touch into one body (a
    /// single solid passes through).
    pub solids: Vec<Solid>,
}

/// Solid Union — the B-rep union of one or more solids in one pass (OCCT
/// general fuse), coplanar faces merged afterwards so two fused blocks are
/// one six-face box; n-ary like `mesh_union`, but a `Solid` is always one
/// body: disjoint operands are red, not a compound.
///
/// # Returns
///
/// The united solid.
///
/// # Panics
///
/// Panics when the list is empty, the operands do not form one connected
/// body, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// shift = unit_x(factor=1.0)
/// blocks = linear_array(geometry=block, direction=shift, count=3)
/// bar = solid_union(solids=blocks)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Solid Union"
)]
#[must_use]
pub fn solid_union(input: SolidUnionIn) -> Solid {
    red(cicada_geom::solid::union_all(&input.solids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solids::support::{
        brep_box, close_rel, config, platform_golden, solid_hash, volume_of, with_kernel,
    };

    #[test]
    fn solid_union_table_cases() {
        let a = || brep_box([0.0; 3], [2.0; 3]);
        let b = || brep_box([1.0, 0.0, 0.0], [2.0; 3]);
        // 8 + 8 − 4.
        let Some(ab) = with_kernel(|| {
            solid_union(SolidUnionIn {
                solids: vec![a(), b()],
            })
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&ab), 12.0, 1e-12));
        let (_, _, faces) = cicada_geom::solid::edges_and_vertices(
            &ab,
            cicada_geom::solid::Deflection::display(&config()),
        )
        .unwrap();
        assert_eq!(faces, 6, "coplanar faces merged: a 3 × 2 × 2 box");
        // Three in one pass: 8 + 8 + 8 − 4 − 4.
        let abc = solid_union(SolidUnionIn {
            solids: vec![a(), b(), brep_box([2.0, 0.0, 0.0], [2.0; 3])],
        });
        assert!(close_rel(volume_of(&abc), 16.0, 1e-12));
        // Touching along a face is one body.
        let touching = solid_union(SolidUnionIn {
            solids: vec![a(), brep_box([2.0, 0.0, 0.0], [2.0; 3])],
        });
        assert!(close_rel(volume_of(&touching), 16.0, 1e-12));
        // One solid passes through.
        assert_eq!(solid_union(SolidUnionIn { solids: vec![a()] }), a());
    }

    #[test]
    #[should_panic(expected = "a union needs at least one solid")]
    fn solid_union_empty_is_red() {
        let _ = solid_union(SolidUnionIn { solids: vec![] });
    }

    #[test]
    fn solid_union_disjoint_bodies_are_red() {
        let Some(()) = with_kernel(|| {
            let outcome = std::panic::catch_unwind(|| {
                solid_union(SolidUnionIn {
                    solids: vec![brep_box([0.0; 3], [1.0; 3]), brep_box([5.0; 3], [1.0; 3])],
                })
            });
            let payload = outcome.expect_err("two bodies are not one solid");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            assert!(
                message.contains("union left 2 solids — a Solid is one body"),
                "{message}"
            );
            assert!(!message.contains("cicada_"), "{message}");
        }) else {
            return;
        };
    }

    proptest::proptest! {
        // Two world-aligned boxes that overlap: |A ∪ B| = |A| + |B| − |A ∩ B|.
        #[test]
        fn property_solid_union_inclusion_exclusion(
            ox in 0.1f64..1.9, oy in 0.1f64..1.9, oz in 0.1f64..1.9,
            sx in 0.5f64..3.0, sy in 0.5f64..3.0, sz in 0.5f64..3.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let extents = [2.0, 2.0, 2.0];
                let out = solid_union(SolidUnionIn {
                    solids: vec![brep_box([0.0; 3], extents), brep_box([ox, oy, oz], [sx, sy, sz])],
                });
                let overlap = crate::meshes::support::overlap_volume([0.0; 3], extents, [ox, oy, oz], [sx, sy, sz]);
                let want = 8.0 + sx * sy * sz - overlap;
                proptest::prop_assert!(close_rel(volume_of(&out), want, 1e-9), "got {} want {}", volume_of(&out), want);
            }
        }
    }

    #[test]
    fn solid_union_determinism_golden_hash() {
        // Two overlapping boxes, exact coordinates: a boolean's canonical
        // bytes, blessed via run-once on win-64 (2026-08-20) after the
        // heap-independence check in `occt/node_set_tests.rs`.
        let Some(bar) = with_kernel(|| {
            solid_union(SolidUnionIn {
                solids: vec![
                    brep_box([0.0; 3], [2.0, 1.0, 1.0]),
                    brep_box([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]),
                ],
            })
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&bar),
            platform_golden("00224f6c88c0ae3068fbac0fc3e53f6223303db1c3a10b8bcfe9ccf040a37f75")
        );
    }
}
