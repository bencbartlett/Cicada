//! The `unit_z` node.

use cicada_core::spatial::Vector;
use cicada_macros::node;

use super::UnitIn;

/// Unit Z — the world z direction, scaled.
///
/// # Examples
///
/// ```cic
/// up = unit_z(factor=3.0)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Unit Z"
)]
#[must_use]
pub fn unit_z(input: UnitIn) -> Vector {
    Vector::new(0.0, 0.0, input.factor)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn unit_z_table() {
        assert_eq!(unit_z(UnitIn { factor: 0.5 }), Vector::new(0.0, 0.0, 0.5));
    }

    proptest::proptest! {
        // The unit node places the factor on its own axis, exactly.
        #[test]
        fn property_unit_z_places_factor(factor in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(unit_z(UnitIn { factor }), Vector::new(0.0, 0.0, factor));
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn unit_z_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        assert_eq!(
            hash(ValueData::Vector(unit_z(UnitIn { factor: 0.5 }))),
            "e28e1b86a745a362445331e3d3aded83354ef778308f5bab9e7ed43467b037f6"
        );
    }
}
