//! The `multiply` node.

use cicada_macros::node;

use super::BinaryIn;

/// Multiply — product of two numbers.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn multiply(input: BinaryIn) -> f64 {
    input.a * input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn multiply_table_cases() {
        let two_three = BinaryIn { a: 2.0, b: 3.0 };
        assert_eq!(multiply(two_three), 6.0);
    }

    proptest::proptest! {
        // IEEE multiplication is commutative, and one is its identity.
        #[test]
        fn property_multiply_commutative(a in -1.0e9..1.0e9_f64, b in -1.0e9..1.0e9_f64) {
            proptest::prop_assert_eq!(
                multiply(BinaryIn { a, b }),
                multiply(BinaryIn { a: b, b: a })
            );
            proptest::prop_assert_eq!(multiply(BinaryIn { a, b: 1.0 }), a);
        }
    }

    #[test]
    fn multiply_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(multiply(BinaryIn { a: 1.5, b: 2.5 })),
            "8fb16814dd81aecf4fb62272ff268ffa7cac28cc1997dfaf1b5b85d39e464f76"
        );
    }
}
