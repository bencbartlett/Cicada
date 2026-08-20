//! The `as_watertight` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

/// Inputs for [`as_watertight`].
#[derive(Ports, Clone, Debug)]
pub struct AsWatertightIn {
    /// The mesh to refine.
    pub mesh: Mesh,
}

/// As Watertight — the checked watertight refinement (docs/08: the
/// mesh-tier solid): every edge shared by exactly two consistently
/// oriented triangles.
///
/// # Returns
///
/// The mesh as a checked watertight solid.
///
/// # Panics
///
/// Panics when the mesh has open or inconsistently oriented edges — red
/// with the count, never a silent pass (wall lesson 13).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// sealed = as_watertight(mesh=block)
/// ```
#[node(category = "Mesh & field", tier = "S", version = 1, gh = none)]
#[must_use]
pub fn as_watertight(input: AsWatertightIn) -> Watertight<Mesh> {
    assert!(
        input.mesh.is_watertight(),
        "as_watertight: mesh ({} triangles) has open or inconsistently oriented \
         edges — not a closed solid",
        input.mesh.triangle_count()
    );
    Watertight(input.mesh)
}

#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::Plane;
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::solids::r#box::{BoxIn, box_};
    use crate::solids::support::config;

    #[test]
    fn as_watertight_accepts_closed_refuses_open() {
        let closed = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
                z: Domain::new(0.0, 1.0),
            },
        )
        .0;
        let refined = as_watertight(AsWatertightIn { mesh: closed });
        assert!(refined.0.is_watertight());
    }

    #[test]
    #[should_panic(expected = "not a closed solid")]
    fn as_watertight_open_mesh_is_red() {
        let open = Mesh::new(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        )
        .expect("valid open mesh");
        let _ = as_watertight(AsWatertightIn { mesh: open });
    }

    proptest::proptest! {
        // as_watertight passes any watertight mesh through unchanged.
        #[test]
        fn property_as_watertight_pass_through(
            dx in 0.01..20.0_f64, dy in 0.01..20.0_f64, dz in 0.01..20.0_f64,
        ) {
            let mesh = box_(
                &config(),
                BoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, dx),
                    y: Domain::new(0.0, dy),
                    z: Domain::new(0.0, dz),
                },
            )
            .0;
            let refined = as_watertight(AsWatertightIn { mesh: mesh.clone() });
            proptest::prop_assert_eq!(refined.0, mesh);
        }
    }

    #[test]
    fn as_watertight_determinism_golden_hash() {
        // Pass-through refinement: the hash is exactly the box golden above.
        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 3.0),
            },
        );
        let refined = as_watertight(AsWatertightIn { mesh: cube.0 });
        let sealed = HashedValue::new(ValueData::Mesh(refined.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "3063b49cbeec12ff1b2dc909b7abe1ffbc060cd66c92f62128c89f7926e42766"
        );
    }
}
