//! The `unit_x` node.

use cicada_core::spatial::Vector;
use cicada_macros::node;

use super::UnitIn;

/// Unit X — the world x direction, scaled.
///
/// # Returns
///
/// The vector (factor, 0, 0).
///
/// # Examples
///
/// ```cic
/// step = unit_x(factor=2.0)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Unit X"
)]
#[must_use]
pub fn unit_x(input: UnitIn) -> Vector {
    Vector::new(input.factor, 0.0, 0.0)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn unit_x_table() {
        assert_eq!(unit_x(UnitIn { factor: 2.0 }), Vector::new(2.0, 0.0, 0.0));
    }

    proptest::proptest! {
        // The unit node places the factor on its own axis, exactly.
        #[test]
        fn property_unit_x_places_factor(factor in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(unit_x(UnitIn { factor }), Vector::new(factor, 0.0, 0.0));
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn unit_x_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        assert_eq!(
            hash(ValueData::Vector(unit_x(UnitIn { factor: 2.0 }))),
            "1b6e3426dcd04d7a833c119bf56008d39f59fac41d63367746e88cae9da50cda"
        );
    }
}
