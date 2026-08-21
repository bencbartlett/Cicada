//! The `series` node.

use cicada_macros::{Ports, node};

use crate::checked_count;

/// Inputs for [`series`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct SeriesIn {
    /// First value.
    #[port(default = 0.0)]
    pub start: f64,
    /// Increment between consecutive values.
    #[port(default = 1.0)]
    pub step: f64,
    /// Number of values.
    pub count: i64,
}

/// Series — an arithmetic sequence of numbers.
///
/// # Returns
///
/// `count` numbers, `start` first, each `step` after the previous.
///
/// # Panics
///
/// Panics when `count` is negative — loud refusal, never a silent empty
/// list (the scheduler turns node panics into red nodes, stage 3) — or
/// above the 2^24 slot ceiling (16,777,216 slots — an unbounded count once
/// aborted the engine on allocation failure instead of going red).
///
/// # Examples
///
/// ```cic
/// xs = series(start=0.0, step=2.5, count=4)
/// ```
#[node(
    category = "Sequences & random",
    tier = "S",
    version = 1,
    gh = "Series"
)]
#[must_use]
pub fn series(input: SeriesIn) -> Vec<f64> {
    let count = checked_count("series", "count", input.count, 0, size_of::<f64>());
    // Per-element multiply (not accumulation) keeps results exact-of-form
    // start + step·i and independent of evaluation order. Counts stay below
    // 2^24 (the slot ceiling), so the cast is loss-free.
    #[allow(clippy::cast_precision_loss)]
    (0..count)
        .map(|i| input.start + input.step * i as f64)
        .collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use std::sync::Arc;

    use cicada_core::value::{HashedValue, List, ValueData};

    use super::*;

    #[test]
    fn table_cases() {
        assert_eq!(
            series(SeriesIn {
                start: 0.0,
                step: 1.0,
                count: 5
            }),
            vec![0.0, 1.0, 2.0, 3.0, 4.0]
        );
        assert_eq!(
            series(SeriesIn {
                start: 2.5,
                step: -0.5,
                count: 3
            }),
            vec![2.5, 2.0, 1.5]
        );
        assert_eq!(
            series(SeriesIn {
                start: 7.0,
                step: 3.0,
                count: 0
            }),
            Vec::<f64>::new()
        );
    }

    #[test]
    #[should_panic(expected = "count must be >= 0")]
    fn negative_count_is_refused_loudly() {
        let _ = series(SeriesIn {
            start: 0.0,
            step: 1.0,
            count: -1,
        });
    }

    #[test]
    #[should_panic(
        expected = "series: count is 100000000000 — above the 16777216 (2^24) slot ceiling"
    )]
    fn absurd_count_is_refused_not_allocated() {
        let _ = series(SeriesIn {
            start: 0.0,
            step: 1.0,
            count: 100_000_000_000,
        });
    }

    proptest::proptest! {
        // Length equals count; each element is exactly start + step·i.
        #[test]
        fn property_shape_and_form(
            start in -1.0e6..1.0e6_f64,
            step in -1.0e3..1.0e3_f64,
            count in 0i64..200,
        ) {
            let out = series(SeriesIn { start, step, count });
            let len = i64::try_from(out.len()).expect("count < 200 fits i64");
            proptest::prop_assert_eq!(len, count);
            for (i, &x) in out.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let want = start + step * i as f64;
                proptest::prop_assert_eq!(x, want);
            }
        }
    }

    // Golden hash of the full output as a Merkle list value — exercises the
    // node AND the list hashing path together. Blessed via run-once.
    #[test]
    fn determinism_golden_hash() {
        let slots = series(SeriesIn {
            start: 0.0,
            step: 0.25,
            count: 4,
        })
        .into_iter()
        .map(|x| Some(HashedValue::new(ValueData::Number(x)).unwrap()))
        .collect();
        let list = HashedValue::new(ValueData::List(List { axis: None, slots })).unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "421cb85a329981b0ac50dbd76467a1af90e0b70c46fc11c4c7b59e45af790316"
        );
        let _ = Arc::clone(&list); // keep Arc in the signature honest
    }
}
