//! The `construct_domain` node.

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

/// Inputs for [`construct_domain`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructDomainIn {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

/// Construct Domain — a numeric interval from its endpoints.
///
/// # Returns
///
/// The interval from `start` to `end` (a decreasing interval is legal).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=4.0)
/// ```
#[node(
    category = "Maths & logic",
    tier = "S",
    version = 1,
    gh = "Construct Domain"
)]
#[must_use]
pub fn construct_domain(input: ConstructDomainIn) -> Domain {
    Domain::new(input.start, input.end)
}

// The construct ∘ deconstruct round-trip exercises both domain nodes and
// lives here with the primary node; `deconstruct_domain.rs` carries its
// own three tests as well.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::maths::deconstruct_domain::{DeconstructDomainIn, deconstruct_domain};

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
        // Construct then deconstruct is the identity on endpoints.
        #[test]
        fn domain_property_roundtrip(start in -1.0e12..1.0e12_f64, end in -1.0e12..1.0e12_f64) {
            let out = deconstruct_domain(DeconstructDomainIn {
                domain: construct_domain(ConstructDomainIn { start, end }),
            });
            proptest::prop_assert_eq!((out.start, out.end), (start, end));
        }
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
        // deconstruct_domain: both outputs through the value model.
        let ends = deconstruct_domain(DeconstructDomainIn { domain });
        let number_hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        assert_eq!(
            number_hash(ends.start),
            "71b099e9be5351c658523316836088b7b65d8d393e485cc825e0ce991ef90f01"
        );
        assert_eq!(
            number_hash(ends.end),
            "bbdac5dc7660e2a6d1f490af14375cb2a0eb92a909a7a7d30730f0ff206775f9"
        );
    }
}
