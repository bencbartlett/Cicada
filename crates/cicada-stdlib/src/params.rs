//! Params & input nodes (docs/08 §Catalog 1). Constructor bindings in the
//! dialect (`amps = slider(value=12.0, min=0.0, max=30.0)`, doc 10 §3);
//! the canvas renders them as widgets, the engine sees plain nodes.
//! Bare literals cover the "Literals" catalog row — a constant is a
//! binding, not a node.

use cicada_core::marshal::AnyValue;
use cicada_macros::{Ports, node};

/// Inputs for [`slider`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct SliderIn {
    /// Current value.
    pub value: f64,
    /// Lower bound.
    #[port(default = 0.0)]
    pub min: f64,
    /// Upper bound.
    #[port(default = 10.0)]
    pub max: f64,
    /// Snap increment for the canvas widget and scrub caching (doc 12);
    /// 0 = continuous. Canvas metadata only — NOT validated at solve
    /// time (an off-step value is legal; exact multiple-of checks on
    /// IEEE doubles would refuse honest values).
    #[port(default = 0.0)]
    pub step: f64,
}

/// Number Slider — a bounded numeric parameter.
///
/// # Panics
///
/// Panics when `value` lies outside `min..=max` or the bounds are
/// inverted — a drifted literal is a loud red, never a silent clamp.
#[node(category = "Params & input", tier = "S", version = 1)]
#[must_use]
pub fn slider(input: SliderIn) -> f64 {
    assert!(
        input.min <= input.max,
        "slider: min {} exceeds max {}",
        input.min,
        input.max
    );
    assert!(
        input.min <= input.value && input.value <= input.max,
        "slider: value {} is outside {}..={}",
        input.value,
        input.min,
        input.max
    );
    input.value
}

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

/// Inputs for [`panel`].
#[derive(Ports, Clone, Debug)]
pub struct PanelIn {
    /// Anything — the panel shows whatever arrives.
    pub data: AnyValue,
}

/// Panel — display sink; shows counts and samples on the canvas.
#[node(category = "Params & input", tier = "S", version = 1)]
pub fn panel(input: PanelIn) {
    // Pure sink: display happens at the viewer (display is an edge,
    // docs/08 rule 9); headless solves just pull the input.
    let _ = input;
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // parameter pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn slider_table_cases() {
        let cases: &[(f64, f64, f64, f64)] = &[
            (5.0, 0.0, 10.0, 0.0),
            (0.0, 0.0, 10.0, 0.5),
            (10.0, 0.0, 10.0, 0.0),
            (-3.0, -5.0, -1.0, 0.0),
        ];
        for &(value, min, max, step) in cases {
            assert_eq!(
                slider(SliderIn {
                    value,
                    min,
                    max,
                    step
                }),
                value
            );
        }
    }

    #[test]
    #[should_panic(expected = "outside")]
    fn slider_out_of_range_is_red() {
        let _ = slider(SliderIn {
            value: 11.0,
            min: 0.0,
            max: 10.0,
            step: 0.0,
        });
    }

    #[test]
    #[should_panic(expected = "min")]
    fn slider_inverted_bounds_are_red() {
        let _ = slider(SliderIn {
            value: 5.0,
            min: 10.0,
            max: 0.0,
            step: 0.0,
        });
    }

    #[test]
    fn toggle_passes_through() {
        assert!(toggle(ToggleIn { value: true }));
        assert!(!toggle(ToggleIn { value: false }));
    }

    #[test]
    fn panel_accepts_any_value_kind() {
        for data in [
            ValueData::Number(1.5),
            ValueData::Text(std::sync::Arc::from("hello")),
            ValueData::Nothing,
        ] {
            panel(PanelIn {
                data: AnyValue(HashedValue::new(data).unwrap()),
            });
        }
    }

    proptest::proptest! {
        // Pass-through is exact for any in-range value.
        #[test]
        fn slider_property_identity(value in -1.0e6..1.0e6_f64) {
            let out = slider(SliderIn {
                value,
                min: -1.0e6,
                max: 1.0e6,
                step: 0.0,
            });
            proptest::prop_assert_eq!(out, value);
        }

        // Toggle pass-through is the identity on both states.
        #[test]
        fn toggle_property_identity(value in proptest::bool::ANY) {
            proptest::prop_assert_eq!(toggle(ToggleIn { value }), value);
        }

        // The panel sink is total: any sealed value is accepted.
        #[test]
        fn panel_property_total(x in -1.0e12..1.0e12_f64) {
            panel(PanelIn {
                data: AnyValue(HashedValue::new(ValueData::Number(x)).unwrap()),
            });
        }
    }

    // Golden hash: the slider's output through the value model.
    #[test]
    fn determinism_golden_hash() {
        let out = slider(SliderIn {
            value: 12.5,
            min: 0.0,
            max: 30.0,
            step: 0.5,
        });
        assert_eq!(
            HashedValue::new(ValueData::Number(out))
                .unwrap()
                .hash()
                .to_hex(),
            "d70452c3371c32d7e65f89aa3027161e96121a2c26bf7c17161cdf6e152577cd"
        );
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
