//! The `mesh_intersection` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`mesh_intersection`].
#[derive(Ports, Clone, Debug)]
pub struct MeshIntersectionIn {
    /// First solid.
    pub a: Watertight<Mesh>,
    /// Second solid.
    pub b: Watertight<Mesh>,
}

/// Mesh Intersection — the intersection of two watertight meshes via
/// Manifold; the result may be the empty solid.
///
/// # Panics
///
/// Panics when Manifold refuses an operand.
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn mesh_intersection(input: MeshIntersectionIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::boolean::intersection(
        &input.a.0, &input.b.0,
    )))
}

// Property coverage: the inclusion–exclusion property in `mesh_union.rs`
// exercises this node.
#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::r#box::{BoxIn, box_};
    use crate::solids::support::config;

    #[test]
    fn mesh_intersection_table_cases() {
        let unit = |origin: f64| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(origin, origin, origin),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 1.0),
                    z: Domain::new(0.0, 1.0),
                },
            )
        };
        let overlap = mesh_intersection(MeshIntersectionIn {
            a: unit(0.0),
            b: unit(0.5),
        });
        assert!((signed_volume(&overlap.0) - 0.125).abs() < 1e-9);
    }

    #[test]
    fn mesh_intersection_determinism_golden_hash() {
        let cube = |origin: f64| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(origin, origin, origin),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 2.0),
                    y: Domain::new(0.0, 2.0),
                    z: Domain::new(0.0, 2.0),
                },
            )
        };
        let overlap = mesh_intersection(MeshIntersectionIn {
            a: cube(0.0),
            b: cube(1.0),
        });
        let sealed = HashedValue::new(ValueData::Mesh(overlap.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "e0ac967780140db5597693d697dbc99cacb43959b3642fd959ece9f707acac80"
        );
    }
}
