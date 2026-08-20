//! The `sqrt` node.

use cicada_macros::node;

use super::UnaryIn;
/// Square Root — the non-negative square root of `x` (IEEE: correctly
/// rounded, so results are identical on every platform).
///
/// # Returns
///
/// `√x`.
///
/// # Panics
///
/// Panics when `x` is negative — the real square root is undefined there
/// (loud refusal, never a silent NaN).
///
/// # Examples
///
/// ```cic
/// side = sqrt(x=2.25)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Square Root"
)]
#[must_use]
pub fn sqrt(input: UnaryIn) -> f64 {
    assert!(
        input.x >= 0.0,
        "sqrt: x must be >= 0 (the real square root is undefined), got {}",
        input.x
    );
    input.x.sqrt()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn sqrt_table_cases() {
        assert_eq!(sqrt(UnaryIn { x: 9.0 }), 3.0);
        assert_eq!(sqrt(UnaryIn { x: 2.25 }), 1.5);
        assert_eq!(sqrt(UnaryIn { x: 0.0 }), 0.0);
        assert_eq!(sqrt(UnaryIn { x: f64::INFINITY }), f64::INFINITY);
        // Correctly rounded by IEEE 754: √2's bit pattern is a constant.
        assert_eq!(sqrt(UnaryIn { x: 2.0 }).to_bits(), 0x3FF6_A09E_667F_3BCD);
    }

    #[test]
    #[should_panic(expected = "x must be >= 0")]
    fn sqrt_of_a_negative_is_red() {
        let _ = sqrt(UnaryIn { x: -1.0 });
    }

    proptest::proptest! {
        // Non-negative, monotone, and squares back within a rounding error.
        #[test]
        fn sqrt_property_squares_back(x in 0.0..1.0e12_f64) {
            let r = sqrt(UnaryIn { x });
            proptest::prop_assert!(r >= 0.0);
            proptest::prop_assert!((r * r - x).abs() <= x * 4.0 * f64::EPSILON);
            proptest::prop_assert!(sqrt(UnaryIn { x: x * 4.0 }) >= r, "monotone");
        }
    }

    #[test]
    fn sqrt_determinism_golden_hash() {
        // An exact dyadic result (√2.25 = 1.5) AND a correctly-rounded
        // irrational one (√2) — both platform-free by the IEEE sqrt contract.
        assert_eq!(
            hex(sqrt(UnaryIn { x: 2.25 })),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
        assert_eq!(
            hex(sqrt(UnaryIn { x: 2.0 })),
            "de3adeb12bfe052927b0302b8d05802ed77dde8658cbcfa844a214dcdc466f7a"
        );
    }
}
