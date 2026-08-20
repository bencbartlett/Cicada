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
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
pub fn deconstruct_domain(input: DeconstructDomainIn) -> DeconstructDomainOut {
    DeconstructDomainOut {
        start: input.domain.start,
        end: input.domain.end,
    }
}

// Tests: the construct/deconstruct round-trip tests (table, property,
// golden hash) live in `construct_domain.rs` — they exercise both nodes.
