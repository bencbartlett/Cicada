//! The `acos` node.

use cicada_macros::node;

use super::UnaryIn;
/// Arccosine — `arccos x`, an angle in radians.
///
/// # Returns
///
/// `arccos x` in radians.
///
/// # Panics
///
/// Panics when `|x| > 1` — the function is undefined there (loud refusal,
/// never a silent NaN).
///
/// # Examples
///
/// ```cic
/// angle = acos(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "ArcCosine")]
#[must_use]
pub fn acos(input: UnaryIn) -> f64 {
    assert!(
        input.x.abs() <= 1.0,
        "acos: x must lie in -1..=1, got {}",
        input.x
    );
    input.x.acos()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn acos_table_cases() {
        assert_eq!(acos(UnaryIn { x: 1.0 }), 0.0, "IEEE pins acos 1 = +0");
        assert!((acos(UnaryIn { x: 0.0 }) - std::f64::consts::FRAC_PI_2).abs() < 1.0e-15);
        assert!((acos(UnaryIn { x: -1.0 }) - std::f64::consts::PI).abs() < 1.0e-15);
    }

    #[test]
    #[should_panic(expected = "x must lie in -1..=1")]
    fn acos_outside_the_unit_interval_is_red() {
        let _ = acos(UnaryIn { x: 1.5 });
    }

    proptest::proptest! {
        // Inverts cos on [0, π] and stays in that range.
        #[test]
        fn acos_property_inverts(x in 0.0..3.1_f64) {
            let a = acos(UnaryIn { x: x.cos() });
            proptest::prop_assert!((a - x).abs() <= 1.0e-7);
            proptest::prop_assert!((0.0..=std::f64::consts::PI).contains(&a));
        }
    }

    #[test]
    fn acos_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(acos(UnaryIn { x: 1.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            acos(UnaryIn { x: 0.3 }).to_bits(),
            acos(UnaryIn { x: 0.3 }).to_bits()
        );
    }
}
