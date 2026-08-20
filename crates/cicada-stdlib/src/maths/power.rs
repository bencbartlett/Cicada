//! The `power` node.

use cicada_macros::node;

use super::BinaryIn;

/// Power — `a` raised to `b` (`^` in expressions).
///
/// # Returns
///
/// `a` raised to the power `b`.
///
/// # Examples
///
/// ```cic
/// kilo = power(a=2.0, b=10.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = "Power")]
#[must_use]
pub fn power(input: BinaryIn) -> f64 {
    input.a.powf(input.b)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn power_table_cases() {
        assert_eq!(power(BinaryIn { a: 2.0, b: 10.0 }), 1024.0);
        assert_eq!(power(BinaryIn { a: 9.0, b: 0.5 }), 3.0);
    }

    proptest::proptest! {
        // IEEE 754 pins pow(x, 0) = 1 and pow(x, 1) = x exactly, on every
        // platform's libm.
        #[test]
        fn property_power_pinned_exponents(a in 0.001f64..1.0e9) {
            proptest::prop_assert_eq!(power(BinaryIn { a, b: 0.0 }), 1.0);
            proptest::prop_assert_eq!(power(BinaryIn { a, b: 1.0 }), a);
        }
    }

    #[test]
    fn power_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            // 2^10: exactly representable AND on the IEEE-pinned powf
            // path — never a platform-libm hash (adversarial review,
            // stage 4).
            hash(power(BinaryIn { a: 2.0, b: 10.0 })),
            "ed155f8b6d76336f8458372211c5918a5ee4d7f5bda82394c99260a55f1cb0a8"
        );
    }
}
