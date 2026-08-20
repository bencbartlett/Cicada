//! The `modulo` node.

use cicada_macros::node;

use super::BinaryIn;

/// Modulo — IEEE remainder of `a / b` (sign follows `a`).
///
/// # Returns
///
/// The remainder of `a / b`, with the sign of `a`.
///
/// # Panics
///
/// `a % 0` is NaN, which value construction refuses — the node goes red.
///
/// # Examples
///
/// ```cic
/// remainder = modulo(a=7.5, b=2.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = "Modulus")]
#[must_use]
pub fn modulo(input: BinaryIn) -> f64 {
    input.a % input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn modulo_table_cases() {
        assert_eq!(modulo(BinaryIn { a: 7.5, b: 2.0 }), 1.5);
        assert_eq!(modulo(BinaryIn { a: -7.5, b: 2.0 }), -1.5, "sign follows a");
    }

    #[test]
    fn modulo_by_zero_output_is_refused_at_value_construction() {
        // The node returns NaN; the value model refuses it — red, not a
        // silent NaN in a cache key.
        let out = modulo(BinaryIn { a: 1.0, b: 0.0 });
        assert!(out.is_nan());
        assert!(HashedValue::new(ValueData::Number(out)).is_err());
    }

    proptest::proptest! {
        // fmod is exact: the remainder never reaches |b|, and its sign
        // follows `a` (or it is zero).
        #[test]
        fn property_modulo_range_and_sign(a in -1.0e9..1.0e9_f64, b in 0.001f64..1.0e6) {
            let r = modulo(BinaryIn { a, b });
            proptest::prop_assert!(r.abs() < b);
            proptest::prop_assert!(r == 0.0 || (r < 0.0) == (a < 0.0));
        }
    }

    #[test]
    fn modulo_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(modulo(BinaryIn { a: 7.5, b: 2.0 })),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
    }
}
