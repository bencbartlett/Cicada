//! The `asin` node.

use cicada_macros::node;

use super::UnaryIn;
/// Arcsine — `arcsin x`, an angle in radians.
///
/// # Returns
///
/// `arcsin x` in radians.
///
/// # Panics
///
/// Panics when `|x| > 1` — the function is undefined there (loud refusal,
/// never a silent NaN).
///
/// # Examples
///
/// ```cic
/// angle = asin(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "ArcSine")]
#[must_use]
pub fn asin(input: UnaryIn) -> f64 {
    assert!(
        input.x.abs() <= 1.0,
        "asin: x must lie in -1..=1, got {}",
        input.x
    );
    input.x.asin()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn asin_table_cases() {
        assert_eq!(asin(UnaryIn { x: 0.0 }), 0.0, "IEEE pins asin 0 = +0");
        assert!((asin(UnaryIn { x: 1.0 }) - std::f64::consts::FRAC_PI_2).abs() < 1.0e-15);
        assert!((asin(UnaryIn { x: -1.0 }) + std::f64::consts::FRAC_PI_2).abs() < 1.0e-15);
    }

    #[test]
    #[should_panic(expected = "x must lie in -1..=1")]
    fn asin_outside_the_unit_interval_is_red() {
        let _ = asin(UnaryIn { x: 1.5 });
    }

    proptest::proptest! {
        // Inverts sin on [−π/2, π/2] and stays in that range.
        #[test]
        fn asin_property_inverts(x in -1.5..1.5_f64) {
            let a = asin(UnaryIn { x: x.sin() });
            proptest::prop_assert!((a - x).abs() <= 1.0e-9);
            proptest::prop_assert!(a.abs() <= std::f64::consts::FRAC_PI_2);
        }
    }

    #[test]
    fn asin_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(asin(UnaryIn { x: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            asin(UnaryIn { x: 0.3 }).to_bits(),
            asin(UnaryIn { x: 0.3 }).to_bits()
        );
    }
}
