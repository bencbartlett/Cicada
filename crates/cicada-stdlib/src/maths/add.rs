//! The `add` node.

use cicada_macros::{Ports, node};

/// Inputs for [`add`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct AddIn {
    /// First addend.
    pub a: f64,
    /// Second addend.
    pub b: f64,
}

/// Add — sum of two numbers.
///
/// # Returns
///
/// The sum `a + b`.
///
/// # Examples
///
/// ```cic
/// total = add(a=1.5, b=2.25)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = "Addition")]
#[must_use]
pub fn add(input: AddIn) -> f64 {
    input.a + input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn add_table_cases() {
        let cases: &[(f64, f64, f64)] = &[
            (1.0, 2.0, 3.0),
            (0.0, 0.0, 0.0),
            (-1.5, 1.5, 0.0),
            (2.5e300, 2.5e300, 5.0e300),
            (0.1, 0.2, 0.300_000_000_000_000_04), // IEEE 754, exactly
        ];
        for &(a, b, want) in cases {
            assert_eq!(add(AddIn { a, b }), want, "add({a}, {b})");
        }
    }

    proptest::proptest! {
        // IEEE 754 addition is commutative (for non-NaN inputs).
        #[test]
        fn add_property_commutative(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            proptest::prop_assert_eq!(add(AddIn { a, b }), add(AddIn { a: b, b: a }));
        }

        // Zero is the additive identity.
        #[test]
        fn add_property_zero_identity(a in -1.0e12..1.0e12_f64) {
            proptest::prop_assert_eq!(add(AddIn { a, b: 0.0 }), a);
        }
    }

    // Golden output hashes through the value model: byte-identical across
    // runs and platforms is a unit test (DECISIONS.md determinism row).
    // Blessed via the run-once path; update only with the diff explained.
    #[test]
    fn add_determinism_golden_hash() {
        let out = HashedValue::new(ValueData::Number(add(AddIn { a: 1.5, b: 2.25 }))).unwrap();
        assert_eq!(
            out.hash().to_hex(),
            "8fb16814dd81aecf4fb62272ff268ffa7cac28cc1997dfaf1b5b85d39e464f76"
        );
    }
}
