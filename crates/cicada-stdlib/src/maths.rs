//! Maths & logic nodes (docs/08 §Catalog 3).

use cicada_core::scalar::Domain;
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
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn add(input: AddIn) -> f64 {
    input.a + input.b
}

/// Inputs for [`construct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructDomainIn {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

/// Construct Domain — a numeric interval from its endpoints.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn construct_domain(input: ConstructDomainIn) -> Domain {
    Domain::new(input.start, input.end)
}

/// Inputs for [`deconstruct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructDomainIn {
    /// The interval.
    pub domain: Domain,
}

/// Outputs of [`deconstruct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructDomainOut {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

/// Deconstruct Domain — the endpoints of an interval.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn deconstruct_domain(input: DeconstructDomainIn) -> DeconstructDomainOut {
    DeconstructDomainOut {
        start: input.domain.start,
        end: input.domain.end,
    }
}

// Test standards per doc 14: table cases + a proptest property + a golden
// blake3 determinism hash, for every stdlib node. Exact float `==` is
// sanctioned here because these nodes' contract IS exact IEEE arithmetic
// (geometry tests use tolerance-aware asserts instead).
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

    #[test]
    fn domain_table_cases() {
        let domain = construct_domain(ConstructDomainIn {
            start: 2.0,
            end: -3.5,
        });
        assert_eq!(
            domain,
            Domain::new(2.0, -3.5),
            "decreasing domains are legal"
        );
        let out = deconstruct_domain(DeconstructDomainIn { domain });
        assert_eq!((out.start, out.end), (2.0, -3.5));
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

        // Construct then deconstruct is the identity on endpoints.
        #[test]
        fn domain_property_roundtrip(start in -1.0e12..1.0e12_f64, end in -1.0e12..1.0e12_f64) {
            let out = deconstruct_domain(DeconstructDomainIn {
                domain: construct_domain(ConstructDomainIn { start, end }),
            });
            proptest::prop_assert_eq!((out.start, out.end), (start, end));
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

    #[test]
    fn domain_determinism_golden_hash() {
        let domain = construct_domain(ConstructDomainIn {
            start: 0.25,
            end: 4.0,
        });
        let out = HashedValue::new(ValueData::Domain(domain)).unwrap();
        assert_eq!(
            out.hash().to_hex(),
            "53346c4847e95756894eabdce84e548c9cf3aec000de291d944a8f7713b99494"
        );
    }
}
