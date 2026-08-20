//! The `unit_y` node.

use cicada_core::spatial::Vector;
use cicada_macros::node;

use super::UnitIn;

/// Unit Y — the world y direction, scaled.
///
/// # Returns
///
/// The vector (0, factor, 0).
///
/// # Examples
///
/// ```cic
/// step = unit_y(factor=2.0)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Unit Y"
)]
#[must_use]
pub fn unit_y(input: UnitIn) -> Vector {
    Vector::new(0.0, input.factor, 0.0)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn unit_y_table() {
        assert_eq!(unit_y(UnitIn { factor: -1.0 }), Vector::new(0.0, -1.0, 0.0));
    }

    proptest::proptest! {
        // The unit node places the factor on its own axis, exactly.
        #[test]
        fn property_unit_y_places_factor(factor in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(unit_y(UnitIn { factor }), Vector::new(0.0, factor, 0.0));
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn unit_y_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        assert_eq!(
            hash(ValueData::Vector(unit_y(UnitIn { factor: -1.0 }))),
            "9f5accc6c03d40db6656244c5438ff823aa1ee28d72372f03461dfe78995d775"
        );
    }
}
