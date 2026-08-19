//! Mesh booleans via the Manifold seam (`manifold-csg`, DECISIONS.md
//! rented-kernels row). f64-native both directions — our `SoA` buffers
//! interleave into `from_mesh_f64` and back with no precision seam
//! (probe-verified: typed `NotManifold` refusal at construction,
//! byte-deterministic output across processes, ~0.1 ms per small boolean).
//!
//! Precondition discipline: callers hold `Watertight<Mesh>` — core's ONE
//! watertight definition. Manifold's own ε-validity is stricter in corner
//! cases; its refusals surface as [`GeomError::Kernel`] with the offending
//! operand named — loud, attributable, never garbage.

use cicada_core::geometry::Mesh;
use manifold_csg::Manifold;

use crate::GeomError;

/// Interleave a core mesh into a Manifold. `which` names the operand in
/// kernel refusals ("cutter 412"), honoring red-with-IDs (docs/12).
fn to_manifold(mesh: &Mesh, which: &str) -> Result<Manifold, GeomError> {
    let indices: Vec<u64> = mesh.indices().iter().map(|&i| u64::from(i)).collect();
    Manifold::from_mesh_f64(mesh.positions(), 3, &indices).map_err(|error| GeomError::Kernel {
        reason: format!("Manifold refused {which}: {error}"),
    })
}

/// A Manifold back into a core mesh. Index widths: Manifold returns u64
/// indices; a result with more than `u32::MAX` vertices refuses loudly
/// (nothing mesh-tier approaches it).
fn from_manifold(manifold: &Manifold, operation: &str) -> Result<Mesh, GeomError> {
    let (positions, n_props, indices) = manifold.to_mesh_f64();
    if n_props != 3 {
        return Err(GeomError::Kernel {
            reason: format!(
                "{operation}: Manifold returned {n_props} properties per vertex (expected 3)"
            ),
        });
    }
    let narrow: Result<Vec<u32>, _> = indices.iter().map(|&i| u32::try_from(i)).collect();
    let Ok(narrow) = narrow else {
        return Err(GeomError::Kernel {
            reason: format!("{operation}: result exceeds u32 vertex indices"),
        });
    };
    Ok(Mesh::new(positions, narrow)?)
}

/// Union of many watertight meshes (empty input → the empty solid).
///
/// # Errors
///
/// [`GeomError::Kernel`] when Manifold refuses an operand or the result.
pub fn union(meshes: &[Mesh]) -> Result<Mesh, GeomError> {
    let manifolds = meshes
        .iter()
        .enumerate()
        .map(|(index, mesh)| to_manifold(mesh, &format!("operand {index}")))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = manifolds.first() else {
        return Ok(Mesh::new(vec![], vec![])?);
    };
    let mut result = first.clone();
    for manifold in &manifolds[1..] {
        result = result.union(manifold);
    }
    from_manifold(&result, "union")
}

/// Subtract every cutter from `mesh` (no cutters → `mesh` unchanged;
/// the result may be the empty solid).
///
/// # Errors
///
/// [`GeomError::Kernel`] when Manifold refuses the mesh, a cutter (named
/// by index), or the result.
pub fn difference(mesh: &Mesh, cutters: &[Mesh]) -> Result<Mesh, GeomError> {
    let mut result = to_manifold(mesh, "the mesh")?;
    for (index, cutter) in cutters.iter().enumerate() {
        let cutter = to_manifold(cutter, &format!("cutter {index}"))?;
        result = result.difference(&cutter);
    }
    from_manifold(&result, "difference")
}

/// Intersection of two watertight meshes (may be the empty solid).
///
/// # Errors
///
/// [`GeomError::Kernel`] when Manifold refuses an operand or the result.
pub fn intersection(a: &Mesh, b: &Mesh) -> Result<Mesh, GeomError> {
    let a = to_manifold(a, "a")?;
    let b = to_manifold(b, "b")?;
    from_manifold(&a.intersection(&b), "intersection")
}

#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::Plane;

    use crate::meshbuild::{box_mesh, signed_volume};

    use super::*;

    const TOL: f64 = 1e-6;

    fn cube(origin: f64, size: f64) -> Mesh {
        box_mesh(
            &Plane::world_xy(),
            Domain::new(origin, origin + size),
            Domain::new(origin, origin + size),
            Domain::new(origin, origin + size),
            TOL,
        )
        .expect("builds")
    }

    #[test]
    fn difference_carves_the_overlap() {
        let out = difference(&cube(0.0, 1.0), &[cube(0.5, 1.0)]).expect("carves");
        assert!(out.is_watertight());
        assert!((signed_volume(&out) - 0.875).abs() < 1e-9);
    }

    #[test]
    fn union_and_intersection_volumes() {
        let u = union(&[cube(0.0, 1.0), cube(0.5, 1.0)]).expect("unions");
        assert!(u.is_watertight());
        assert!((signed_volume(&u) - 1.875).abs() < 1e-9);
        let i = intersection(&cube(0.0, 1.0), &cube(0.5, 1.0)).expect("intersects");
        assert!((signed_volume(&i) - 0.125).abs() < 1e-9);
    }

    #[test]
    fn empty_cases_are_legal() {
        let empty_union = union(&[]).expect("empty union is the empty solid");
        assert_eq!(empty_union.triangle_count(), 0);
        // Disjoint intersection → empty solid, not an error.
        let disjoint = intersection(&cube(0.0, 1.0), &cube(5.0, 1.0)).expect("disjoint");
        assert_eq!(disjoint.triangle_count(), 0);
        assert!(disjoint.is_watertight(), "empty mesh is watertight");
    }

    #[test]
    fn non_manifold_operand_is_a_named_loud_refusal() {
        let open = Mesh::new(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        )
        .expect("valid open mesh");
        let error = difference(&cube(0.0, 1.0), &[open]).expect_err("must refuse");
        let GeomError::Kernel { reason } = &error else {
            panic!("kernel refusal, got {error:?}")
        };
        assert!(reason.contains("cutter 0"), "operand named: {reason}");
    }

    #[test]
    fn determinism_same_inputs_same_buffers() {
        let a = difference(&cube(0.0, 1.0), &[cube(0.5, 1.0)]).expect("carves");
        let b = difference(&cube(0.0, 1.0), &[cube(0.5, 1.0)]).expect("carves");
        assert_eq!(
            a, b,
            "byte-identical across runs (probe-verified across processes)"
        );
    }
}
