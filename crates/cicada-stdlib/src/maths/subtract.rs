//! The `subtract` node.

use cicada_macros::node;

use super::BinaryIn;

/// Subtract — difference of two numbers.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn subtract(input: BinaryIn) -> f64 {
    input.a - input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn subtract_table_cases() {
        let two_three = BinaryIn { a: 2.0, b: 3.0 };
        assert_eq!(subtract(two_three), -1.0);
    }

    proptest::proptest! {
        // IEEE subtraction is exactly anti-symmetric (rounding is
        // sign-symmetric), and zero is its right identity.
        #[test]
        fn property_subtract_antisymmetry(a in -1.0e9..1.0e9_f64, b in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(
                subtract(BinaryIn { a, b }),
                -subtract(BinaryIn { a: b, b: a })
            );
            proptest::prop_assert_eq!(subtract(BinaryIn { a, b: 0.0 }), a);
        }
    }

    #[test]
    fn subtract_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(subtract(BinaryIn { a: 7.5, b: 2.25 })),
            "ca9cbb2b358bc696b112f70b6377e8e54a72fabc1b4a7603655a0cc8d37f406d"
        );
    }
}
