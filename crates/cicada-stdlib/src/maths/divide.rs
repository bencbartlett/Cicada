//! The `divide` node.

use cicada_macros::node;

use super::BinaryIn;

/// Divide — quotient of two numbers (IEEE: dividing by zero yields ±∞).
///
/// # Examples
///
/// ```cic
/// ratio = divide(a=7.0, b=2.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = "Division")]
#[must_use]
pub fn divide(input: BinaryIn) -> f64 {
    input.a / input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::maths::multiply::multiply;

    #[test]
    fn divide_table_cases() {
        assert_eq!(divide(BinaryIn { a: 7.0, b: 2.0 }), 3.5);
        assert_eq!(divide(BinaryIn { a: 1.0, b: 0.0 }), f64::INFINITY);
    }

    proptest::proptest! {
        // Dividing by one is exact, and divide-then-multiply lands within
        // one rounding step of `a` (two correctly-rounded IEEE ops).
        #[test]
        fn property_divide_roundtrip(a in -1.0e9..1.0e9_f64, b in 0.001f64..1.0e9) {
            proptest::prop_assert_eq!(divide(BinaryIn { a, b: 1.0 }), a);
            let back = multiply(BinaryIn { a: divide(BinaryIn { a, b }), b });
            proptest::prop_assert!((back - a).abs() <= 1e-12 * a.abs().max(1.0));
        }
    }

    #[test]
    fn divide_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(divide(BinaryIn { a: 7.0, b: 2.0 })),
            "b69e5ba382a20ddf8b8873de846ca57fb12935d86345d12513254b517aec8037"
        );
    }
}
