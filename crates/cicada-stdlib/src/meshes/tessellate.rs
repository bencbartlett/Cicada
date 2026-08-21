//! The `tessellate` node (v0.1 item 3 WP-C): the explicit B-rep → mesh
//! bridge (docs/08 §8; DECISIONS.md row 42).

use cicada_core::geometry::{Mesh, Solid, Watertight};
use cicada_geom::solid::Deflection;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`tessellate`].
#[derive(Ports, Clone, Debug)]
pub struct TessellateIn {
    /// The B-rep solid to mesh.
    pub solid: Solid,
    /// The chord deviation: the largest distance between the mesh and the
    /// true surface, in document units (the display uses 0.02 mm). Held to
    /// the node's budget for the part's size — never finer than 1000
    /// facets per full turn at the solid's largest extent, about one
    /// 400,000th of it (0.01 is admitted up to a 4 m part).
    #[port(default = 0.01, dimension = length)]
    pub deflection: f64,
    /// The angular deviation between adjacent facets, in radians (0.1 rad
    /// ≈ 5.7°, about 63 facets per full turn; the budget's floor is
    /// 2π/1000 ≈ 0.0063 rad).
    #[port(default = 0.1, dimension = angle)]
    pub angle: f64,
}

/// Tessellate — mesh a B-rep solid into the mesh tier's watertight solid:
/// OCCT's mesher at the given chord and angular deflections, per-face
/// vertices welded, the result checked watertight and accepted by
/// Manifold — so it feeds `mesh_union` / `mesh_difference` / the exporters
/// directly. The explicit, costed bridge (docs/08 rule 8): a `Solid` never
/// becomes a mesh on its own, and the cost is bounded — a request finer
/// than the budget for the part's size is refused before the mesher runs
/// (below it the mesher's memory grows without limit, in one kernel call
/// nothing can interrupt).
///
/// # Returns
///
/// The welded, watertight mesh of the solid.
///
/// # Panics
///
/// Panics when the deflection is below 1e-7 or the angle below 1e-12 rad
/// (the kernel's own floors: the mesher refuses anything finer), when
/// either is finer than the budget for this solid's size (1000 facets per
/// full turn at its largest extent — the message names the floors for
/// the part), when the mesher fails or its output is not closed after
/// welding, when Manifold refuses the welded mesh, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// ball = sphere(radius=1.5)
/// shell = tessellate(solid=ball, deflection=0.05)
/// ```
// `version = 2`: the budget (requests once admitted are now refused) and
// the dropped tolerance slot of the memo key — the node never read the
// project tolerance, so `uses_tolerance` only invalidated every
// tessellation on a tolerance change (review finding 6).
#[node(category = "Mesh & field", tier = "1", version = 2, gh = "Mesh Brep")]
#[must_use]
pub fn tessellate(input: TessellateIn) -> Watertight<Mesh> {
    let deflection = red(Deflection::new(input.deflection, input.angle));
    let tessellation = red(cicada_geom::solid::tessellate_within_budget(
        &input.solid,
        deflection,
    ));
    // Manifold acceptance: the mesh tier's booleans must take this mesh
    // as-is (a structurally watertight mesh can still be non-manifold —
    // a pinched vertex — which Manifold refuses loudly).
    red(cicada_geom::boolean::accepted(&tessellation.mesh.0));
    tessellation.mesh
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::support::{brep_box, close_rel, config, plane_at, with_kernel};

    #[test]
    fn tessellate_table_cases() {
        // A box: twelve triangles whatever the deflection.
        let Some(cube) = with_kernel(|| {
            tessellate(TessellateIn {
                solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                deflection: 0.01,
                angle: 0.1,
            })
        }) else {
            return;
        };
        assert!(cube.0.is_watertight());
        assert_eq!(cube.0.triangle_count(), 12);
        assert_eq!(cube.0.vertex_count(), 8);
        assert!(close_rel(signed_volume(&cube.0), 6.0, 1e-12));
        // A sphere: finer deflection, more triangles, volume closer to
        // 4/3 π r³ — and watertight both times.
        let ball =
            cicada_geom::solid::sphere(&plane_at(0.0, 0.0, 0.0), 2.0, config().tol()).unwrap();
        let coarse = tessellate(TessellateIn {
            solid: ball.clone(),
            deflection: 0.5,
            angle: 0.5,
        });
        let fine = tessellate(TessellateIn {
            solid: ball,
            deflection: 0.005,
            angle: 0.05,
        });
        assert!(coarse.0.is_watertight() && fine.0.is_watertight());
        assert!(fine.0.triangle_count() > coarse.0.triangle_count());
        let exact = 4.0 / 3.0 * PI * 8.0;
        let coarse_error = (signed_volume(&coarse.0) - exact).abs();
        let fine_error = (signed_volume(&fine.0) - exact).abs();
        assert!(fine_error < coarse_error, "{fine_error} vs {coarse_error}");
        assert!(fine_error / exact < 5e-3);
        // The result feeds the mesh tier's booleans (Manifold accepted it).
        let carved = crate::meshes::mesh_difference::mesh_difference(
            crate::meshes::mesh_difference::MeshDifferenceIn {
                mesh: cube.clone(),
                cutters: vec![fine],
            },
        );
        assert!(carved.0.is_watertight());
    }

    #[test]
    #[should_panic(expected = "linear_deflection")]
    fn tessellate_below_the_kernel_floor_is_red() {
        let _ = tessellate(TessellateIn {
            solid: Solid::from_canonical_bytes(
                cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec(),
            )
            .unwrap(),
            deflection: 1e-9,
            angle: 0.1,
        });
    }

    /// The review's hostile case (finding 2): a unit sphere at the kernel's
    /// bare floor, 1e-7 — the mesher accepted it, grew past 23 GB in 25 s
    /// and never finished, in one uninterruptible call. The node refuses
    /// it before the mesher runs, typed, with the floors for THIS part in
    /// the message; the test's own speed is the evidence that nothing was
    /// meshed (the mesher's run is the part that does not finish).
    #[test]
    fn tessellate_finer_than_the_budget_is_red_before_the_mesher_runs() {
        let Some(()) = with_kernel(|| {
            let ball =
                cicada_geom::solid::sphere(&plane_at(0.0, 0.0, 0.0), 1.0, config().tol()).unwrap();
            for (deflection, angle) in [(1e-7, 0.1), (0.01, 1e-6)] {
                let outcome = std::panic::catch_unwind(|| {
                    tessellate(TessellateIn {
                        solid: ball.clone(),
                        deflection,
                        angle,
                    })
                });
                let payload = outcome.expect_err("finer than the budget must be red");
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_default();
                for expected in [
                    "finer than the budget",
                    "1000 facets per full turn",
                    "2 across",
                    "coarsen the request",
                ] {
                    assert!(message.contains(expected), "{message}");
                }
            }
        }) else {
            return;
        };
    }

    proptest::proptest! {
        // Boxes anywhere at any sane deflection: always twelve triangles,
        // always the exact volume.
        #[test]
        fn property_tessellate_boxes_are_exact(
            sx in 0.1f64..20.0, sy in 0.1f64..20.0, sz in 0.1f64..20.0,
            deflection in 0.001f64..1.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = tessellate(TessellateIn {
                    solid: brep_box([0.0; 3], [sx, sy, sz]),
                    deflection,
                    angle: 0.1,
                });
                proptest::prop_assert_eq!(out.0.triangle_count(), 12);
                proptest::prop_assert!(close_rel(signed_volume(&out.0), sx * sy * sz, 1e-9));
            }
        }
    }

    #[test]
    fn tessellate_determinism_golden_hash() {
        // A box's tessellation: eight exact corners, twelve triangles — a
        // transcendental-free mesh. Blessed via run-once on win-64
        // (2026-08-20); unchanged by the budget (an admitted request meshes
        // exactly as before).
        let Some(cube) = with_kernel(|| {
            tessellate(TessellateIn {
                solid: brep_box([0.0; 3], [1.0, 2.0, 3.0]),
                deflection: 0.01,
                angle: 0.1,
            })
        }) else {
            return;
        };
        let sealed = HashedValue::new(ValueData::Mesh(cube.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            crate::solids::support::platform_golden(
                "0a0565a3a8cf66714507a4e9c94fb6f4e8d437f4131e2b0793814d9f25dc2d2b"
            )
        );
    }
}
