//! Maths & logic nodes (docs/08 §Catalog 3).

use cicada_core::spec::{NodeSpec, PortSpec};

/// Inputs for [`add`].
///
/// The node ABI is struct-in/struct-out (DECISIONS.md): one input struct with
/// named fields; the field names ARE the port names in the catalog, canvas,
/// and dialect kwargs. Stage 1's `#[derive(Ports)]` will reflect this struct
/// into [`ADD_SPEC`] automatically; until then the two are kept in sync by
/// hand.
#[derive(Clone, Copy, Debug)]
pub struct AddIn {
    /// First addend.
    pub a: f64,
    /// Second addend.
    pub b: f64,
}

/// Add — sum of two numbers.
///
/// Stage-0 stub: the first node through the registry → catalog pipeline,
/// hand-registered until `#[node]` lands in stage 1. Kept deliberately
/// trivial — its job is to prove the end-to-end path (function + spec +
/// tests + catalog line), not to be useful yet. Single-output nodes return
/// a bare value; the port is named `out` (docs/08).
#[must_use]
pub fn add(input: AddIn) -> f64 {
    input.a + input.b
}

/// The spec `#[node]` will generate for [`add`] in stage 1.
pub static ADD_SPEC: NodeSpec = NodeSpec {
    name: "add",
    title: "Add",
    description: "Sum of two numbers.",
    category: "Maths & logic",
    inputs: &[
        PortSpec {
            name: "a",
            ty: "Number",
            default: None,
        },
        PortSpec {
            name: "b",
            ty: "Number",
            default: None,
        },
    ],
    outputs: &[PortSpec {
        name: "out",
        ty: "Number",
        default: None,
    }],
};

// Test standards per doc 14: table cases + a proptest property + a golden
// blake3 determinism hash, for every stdlib node. Raw float `==` is
// sanctioned only in hash/determinism tests (DECISIONS.md tolerance row);
// these stage-0 tests compare exact IEEE 754 results on purpose.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn table_cases() {
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

    proptest! {
        // IEEE 754 addition is commutative (for non-NaN inputs).
        #[test]
        fn property_commutative(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            prop_assert_eq!(add(AddIn { a, b }), add(AddIn { a: b, b: a }));
        }

        // Zero is the additive identity.
        #[test]
        fn property_zero_identity(a in -1.0e12..1.0e12_f64) {
            prop_assert_eq!(add(AddIn { a, b: 0.0 }), a);
        }
    }

    // Golden output hash: byte-identical across runs and platforms is a unit
    // test (DECISIONS.md determinism row). Update only through the blessed
    // path with the diff explained (doc 14 §Testing standards; stage-0
    // blessed path = run once, copy the actual, explain in the commit).
    #[test]
    fn determinism_golden_hash() {
        let out = add(AddIn { a: 1.5, b: 2.25 });
        let hash = blake3::hash(&out.to_le_bytes()).to_hex();
        assert_eq!(
            hash.as_str(),
            "f4faa1d667369e19dca46b1134ad7e4bce1f568becd66eefd7a545c35aec8fd9",
            "add(1.5, 2.25) output bytes hashed differently"
        );
    }
}
