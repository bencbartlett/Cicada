//! Test helpers shared by the boolean nodes' tests: world-aligned boxes and
//! the analytic volume of two boxes' overlap, so each boolean's property
//! test checks Manifold against measure theory without leaning on a
//! sibling boolean for its expected value.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point};

use crate::meshes::mesh_box::{MeshBoxIn, mesh_box};
use crate::solids::support::config;

/// A world-aligned box with its min corner at `origin` and the given
/// positive extents.
pub(crate) fn aligned_box(origin: [f64; 3], extents: [f64; 3]) -> Watertight<Mesh> {
    mesh_box(
        &config(),
        MeshBoxIn {
            plane: Plane {
                origin: Point::new(origin[0], origin[1], origin[2]),
                ..Plane::world_xy()
            },
            x: Domain::new(0.0, extents[0]),
            y: Domain::new(0.0, extents[1]),
            z: Domain::new(0.0, extents[2]),
        },
    )
}

/// The volume of the overlap of two world-aligned boxes (min corner +
/// extents each): the product of the per-axis interval overlaps, zero when
/// they are apart.
pub(crate) fn overlap_volume(
    origin_a: [f64; 3],
    extents_a: [f64; 3],
    origin_b: [f64; 3],
    extents_b: [f64; 3],
) -> f64 {
    (0..3)
        .map(|axis| {
            let lo = origin_a[axis].max(origin_b[axis]);
            let hi = (origin_a[axis] + extents_a[axis]).min(origin_b[axis] + extents_b[axis]);
            (hi - lo).max(0.0)
        })
        .product()
}
