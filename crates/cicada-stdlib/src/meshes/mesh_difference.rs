//! The `mesh_difference` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`mesh_difference`].
#[derive(Ports, Clone, Debug)]
pub struct MeshDifferenceIn {
    /// The solid to carve.
    pub mesh: Watertight<Mesh>,
    /// The solids to subtract (the wall's carve: one part, its cutters).
    pub cutters: Vec<Watertight<Mesh>>,
}

/// Mesh Difference — subtract every cutter from a mesh via Manifold; the
/// result may be the empty solid. Lift with `each()` to carve per part
/// (`mesh_difference(mesh=each(frusta), cutters=each(cutter_groups))`).
///
/// # Panics
///
/// Panics when Manifold refuses the mesh or a cutter (named by index).
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn mesh_difference(input: MeshDifferenceIn) -> Watertight<Mesh> {
    let cutters: Vec<Mesh> = input.cutters.into_iter().map(|w| w.0).collect();
    Watertight(red(cicada_geom::boolean::difference(
        &input.mesh.0,
        &cutters,
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
    fn mesh_difference_table_cases() {
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
        let carved = mesh_difference(MeshDifferenceIn {
            mesh: unit(0.0),
            cutters: vec![unit(0.5)],
        });
        assert!((signed_volume(&carved.0) - 0.875).abs() < 1e-9);
    }

    // The carve's determinism is a golden hash: same inputs → the same
    // bytes out of Manifold, across runs and platforms (the stage-6 corpus
    // extends this to the full wall).
    #[test]
    fn boolean_determinism_golden_hash() {
        let base = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 2.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 2.0),
            },
        );
        // An arithmetic-only cutter: a unit box punched through the top
        // face (transcendental-input goldens are forbidden, see above).
        let cutter = box_(
            &config(),
            BoxIn {
                plane: Plane {
                    origin: Point::new(0.5, 0.5, 1.0),
                    ..Plane::world_xy()
                },
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
                z: Domain::new(0.0, 1.5),
            },
        );
        let carved = mesh_difference(MeshDifferenceIn {
            mesh: base,
            cutters: vec![cutter],
        });
        assert!((signed_volume(&carved.0) - 7.0).abs() < 1e-9);
        let sealed = HashedValue::new(ValueData::Mesh(carved.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "9db199d7450fc730d16600090943d9253066a31ad47809ced55843efb730df74"
        );
    }
}
