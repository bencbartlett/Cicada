//! The `toggle` node.

use cicada_macros::{Ports, node};

/// Inputs for [`toggle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ToggleIn {
    /// Current state.
    pub value: bool,
}

/// Boolean Toggle — an on/off parameter.
#[node(category = "Params & input", tier = "S", version = 1)]
#[must_use]
pub fn toggle(input: ToggleIn) -> bool {
    input.value
}

#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn toggle_passes_through() {
        assert!(toggle(ToggleIn { value: true }));
        assert!(!toggle(ToggleIn { value: false }));
    }

    proptest::proptest! {
        // Toggle pass-through is the identity on both states.
        #[test]
        fn toggle_property_identity(value in proptest::bool::ANY) {
            proptest::prop_assert_eq!(toggle(ToggleIn { value }), value);
        }
    }

    // Golden hash: the toggle's output through the value model. Panel has
    // no counterpart — it returns unit by contract (display is an edge,
    // docs/08 rule 9), so there is no output value to hash.
    #[test]
    fn toggle_determinism_golden_hash() {
        let out = toggle(ToggleIn { value: true });
        assert_eq!(
            HashedValue::new(ValueData::Boolean(out))
                .unwrap()
                .hash()
                .to_hex(),
            "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552"
        );
    }
}
