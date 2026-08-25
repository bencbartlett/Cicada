//! The `slider` node.

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
    /// Scrub caching (doc 12 §Speculative warming, v0.1 item 5): pre-solve
    /// the step-quantized positions while the app is idle, nearest the
    /// current value first, so dragging later is a cache read (the buffer
    /// bar under the slider shows the warm span). Canvas metadata only —
    /// the output is `value` regardless; offered only while `step > 0`,
    /// the range has at most 32 positions, and `min`, `max` and `step` are
    /// literals (the session refuses the toggle otherwise, and warms
    /// nothing for a hand-written `scrub=True` on such a slider).
    #[port(default = false)]
    pub scrub: bool,
}

/// Number Slider — a bounded numeric parameter.
///
/// # Returns
///
/// The current value, within `min..=max`.
///
/// # Panics
///
/// Panics when `value` lies outside `min..=max` or the bounds are
/// inverted — a drifted literal is a loud red, never a silent clamp.
///
/// # Examples
///
/// ```cic
/// amps = slider(value=12.0, min=0.0, max=30.0, step=0.5)
/// ```
#[node(
    category = "Params & input",
    tier = "S",
    version = 2,
    gh = "Number Slider"
)]
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

#[cfg(test)]
#[allow(clippy::float_cmp)] // parameter pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn slider_table_cases() {
        // (value, min, max, step, scrub): `scrub` is canvas metadata and
        // never touches the output.
        let cases: &[(f64, f64, f64, f64, bool)] = &[
            (5.0, 0.0, 10.0, 0.0, false),
            (0.0, 0.0, 10.0, 0.5, false),
            (10.0, 0.0, 10.0, 0.0, false),
            (-3.0, -5.0, -1.0, 0.0, false),
            (2.0, 0.5, 5.0, 0.25, true),
        ];
        for &(value, min, max, step, scrub) in cases {
            assert_eq!(
                slider(SliderIn {
                    value,
                    min,
                    max,
                    step,
                    scrub,
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
            scrub: false,
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
            scrub: false,
        });
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
                scrub: false,
            });
            proptest::prop_assert_eq!(out, value);
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
            scrub: true,
        });
        assert_eq!(
            HashedValue::new(ValueData::Number(out))
                .unwrap()
                .hash()
                .to_hex(),
            "d70452c3371c32d7e65f89aa3027161e96121a2c26bf7c17161cdf6e152577cd"
        );
    }
}
