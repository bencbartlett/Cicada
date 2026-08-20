//! The `cos` node.

use cicada_macros::node;

use super::AngleIn;
/// Cosine — `cos x` for an angle in radians.
///
/// # Returns
///
/// `cos x`.
///
/// # Examples
///
/// ```cic
/// value = cos(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Cosine")]
#[must_use]
pub fn cos(input: AngleIn) -> f64 {
    input.x.cos()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn cos_table_cases() {
        assert_eq!(cos(AngleIn { x: 0.0 }), 1.0, "IEEE pins cos 0 = 1");
        assert!(
            cos(AngleIn {
                x: std::f64::consts::FRAC_PI_2
            })
            .abs()
                < 1.0e-15
        );
        assert!(
            (cos(AngleIn {
                x: std::f64::consts::PI
            }) + 1.0)
                .abs()
                < 1.0e-15
        );
    }

    proptest::proptest! {
        // Bounded, even, and on the unit circle with sin.
        #[test]
        fn cos_property_identities(x in -10.0..10.0_f64) {
            let c = cos(AngleIn { x });
            proptest::prop_assert!(c.abs() <= 1.0);
            proptest::prop_assert_eq!(c, cos(AngleIn { x: -x }), "even");
            proptest::prop_assert!((c * c + x.sin() * x.sin() - 1.0).abs() <= 1.0e-14);
        }
    }

    #[test]
    fn cos_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(cos(AngleIn { x: 0.0 })),
            "2f4e052fbfa58c8bfc1aefe1b236c4ad7383dd3855684a2f830d183393cbaebf"
        );
        assert_eq!(
            cos(AngleIn { x: 0.7 }).to_bits(),
            cos(AngleIn { x: 0.7 }).to_bits()
        );
    }
}
