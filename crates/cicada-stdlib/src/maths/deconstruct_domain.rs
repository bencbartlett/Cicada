//! The `deconstruct_domain` node.

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

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
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=2.0, end=5.0)
/// lo, hi = deconstruct_domain(domain=span)
/// ```
#[node(
    category = "Maths & logic",
    tier = "S",
    version = 1,
    gh = "Deconstruct Domain"
)]
#[must_use]
pub fn deconstruct_domain(input: DeconstructDomainIn) -> DeconstructDomainOut {
    DeconstructDomainOut {
        start: input.domain.start,
        end: input.domain.end,
    }
}

// The construct ∘ deconstruct round-trip (from the other side) also lives
// in `construct_domain.rs`; the three tests below are this node's own.
#[cfg(test)]
#[allow(clippy::float_cmp)] // exact endpoint pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::maths::construct_domain::{ConstructDomainIn, construct_domain};

    #[test]
    fn deconstruct_domain_table_cases() {
        let ends = |start, end| {
            let out = deconstruct_domain(DeconstructDomainIn {
                domain: Domain::new(start, end),
            });
            (out.start, out.end)
        };
        assert_eq!(ends(0.0, 4.0), (0.0, 4.0));
        assert_eq!(
            ends(2.0, -3.5),
            (2.0, -3.5),
            "a decreasing domain keeps its order — no silent normalization"
        );
        assert_eq!(
            ends(1.25, 1.25),
            (1.25, 1.25),
            "an empty domain has equal endpoints"
        );
        assert_eq!(ends(-1.0e12, 1.0e-12), (-1.0e12, 1.0e-12));
    }

    proptest::proptest! {
        // Deconstruct then construct is the identity on the domain — the
        // inverse direction of the round-trip in `construct_domain.rs`.
        #[test]
        fn deconstruct_domain_property_roundtrip(
            start in -1.0e12..1.0e12_f64,
            end in -1.0e12..1.0e12_f64,
        ) {
            let domain = Domain::new(start, end);
            let out = deconstruct_domain(DeconstructDomainIn { domain });
            proptest::prop_assert_eq!(
                construct_domain(ConstructDomainIn { start: out.start, end: out.end }),
                domain
            );
        }
    }

    // Both outputs through the value model (arithmetic-exact inputs;
    // blessed via run-once).
    #[test]
    fn deconstruct_domain_determinism_golden_hash() {
        let out = deconstruct_domain(DeconstructDomainIn {
            domain: Domain::new(-1.5, 2.75),
        });
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        assert_eq!(
            hash(out.start),
            "c6c4cb011bfb7cb49d49a95f462a353aa164086acdb4e934f9580845bbf4533c"
        );
        assert_eq!(
            hash(out.end),
            "dbab1563ec897f6023b0dd1452488558e1dd730c003529d9fa23f110a6fddae0"
        );
    }
}
