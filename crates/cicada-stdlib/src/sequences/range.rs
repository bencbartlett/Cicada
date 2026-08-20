//! The `range` node.

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

/// Inputs for [`range`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RangeIn {
    /// The interval to divide (decreasing domains count down).
    pub domain: Domain,
    /// Number of equal steps; the output has `steps + 1` values.
    pub steps: i64,
}

/// Range — `steps + 1` evenly spaced numbers across a domain, both ends
/// included exactly (`series` when you know the step instead).
///
/// # Returns
///
/// `domain.start`, then `steps - 1` interior values, then `domain.end`.
///
/// # Panics
///
/// Panics when `steps < 1` — a domain cannot be divided into no steps.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=1.0)
/// ticks = range(domain=span, steps=4)
/// ```
#[node(category = "Sequences & random", tier = "1", version = 1, gh = "Range")]
#[must_use]
pub fn range(input: RangeIn) -> Vec<f64> {
    assert!(
        input.steps >= 1,
        "range: steps must be >= 1, got {}",
        input.steps
    );
    let Domain { start, end } = input.domain;
    let span = end - start;
    // Per-element form (never accumulation) keeps every value independent
    // of evaluation order; the ends are the domain's own numbers, exactly.
    #[allow(clippy::cast_precision_loss)] // step counts are far below 2^53
    let steps = input.steps as f64;
    (0..=input.steps)
        .map(|i| {
            if i == 0 {
                start
            } else if i == input.steps {
                end
            } else {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f64 / steps;
                span.mul_add(t, start)
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use cicada_core::marshal::IntoValue;

    use super::*;

    #[test]
    fn range_table_cases() {
        assert_eq!(
            range(RangeIn {
                domain: Domain::new(0.0, 1.0),
                steps: 4
            }),
            vec![0.0, 0.25, 0.5, 0.75, 1.0]
        );
        assert_eq!(
            range(RangeIn {
                domain: Domain::new(10.0, 0.0),
                steps: 2
            }),
            vec![10.0, 5.0, 0.0],
            "decreasing domains count down"
        );
        assert_eq!(
            range(RangeIn {
                domain: Domain::new(-1.0, 1.0),
                steps: 1
            }),
            vec![-1.0, 1.0],
            "one step is just the two ends"
        );
        assert_eq!(
            range(RangeIn {
                domain: Domain::new(2.5, 2.5),
                steps: 3
            }),
            vec![2.5; 4],
            "an empty domain repeats its value"
        );
    }

    #[test]
    #[should_panic(expected = "steps must be >= 1")]
    fn range_zero_steps_is_red() {
        let _ = range(RangeIn {
            domain: Domain::new(0.0, 1.0),
            steps: 0,
        });
    }

    proptest::proptest! {
        // steps + 1 values; the ends are exactly the domain's; monotone in
        // the domain's direction; evenly spaced within a rounding error.
        #[test]
        fn range_property_inclusive_even_spacing(
            start in -1.0e6..1.0e6_f64,
            end in -1.0e6..1.0e6_f64,
            steps in 1i64..200,
        ) {
            let out = range(RangeIn { domain: Domain::new(start, end), steps });
            let len = i64::try_from(out.len()).expect("steps < 200 fits i64");
            proptest::prop_assert_eq!(len, steps + 1);
            proptest::prop_assert_eq!(out[0], start);
            proptest::prop_assert_eq!(*out.last().expect("non-empty"), end);
            #[allow(clippy::cast_precision_loss)]
            let step = (end - start) / steps as f64;
            for pair in out.windows(2) {
                let gap = pair[1] - pair[0];
                proptest::prop_assert!((gap - step).abs() <= 1.0e-9 * (1.0 + step.abs()));
            }
        }
    }

    // Golden hash of the sealed list — dyadic inputs, exact values.
    #[test]
    fn range_determinism_golden_hash() {
        let out = range(RangeIn {
            domain: Domain::new(0.0, 1.0),
            steps: 4,
        });
        assert_eq!(
            out.into_value().unwrap().hash().to_hex(),
            "52a83105d41ae11bf183a3692abd280ded81dc943a627fbf55177f1f56403518"
        );
    }
}
