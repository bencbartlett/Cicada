//! The `average` node.

use cicada_macros::node;

use super::ReduceIn;
/// Average — the arithmetic mean of a list of numbers (summed left to
/// right, then divided by the count).
///
/// # Returns
///
/// The mean.
///
/// # Panics
///
/// Panics when the list is empty — the mean of nothing is undefined (loud
/// refusal, never a silent zero or NaN).
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 6.0]
/// mean = average(list=xs)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Average")]
#[must_use]
pub fn average(input: ReduceIn) -> f64 {
    assert!(
        !input.list.is_empty(),
        "average: the list is empty — the mean of nothing is undefined"
    );
    #[allow(clippy::cast_precision_loss)] // list lengths are far below 2^53
    let count = input.list.len() as f64;
    input.list.iter().sum::<f64>() / count
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn average_table_cases() {
        assert_eq!(
            average(ReduceIn {
                list: vec![1.0, 2.0, 3.0, 6.0]
            }),
            3.0
        );
        assert_eq!(
            average(ReduceIn { list: vec![7.5] }),
            7.5,
            "a singleton is itself"
        );
        assert_eq!(
            average(ReduceIn {
                list: vec![-2.0, 2.0]
            }),
            0.0
        );
        assert_eq!(
            average(ReduceIn {
                list: vec![1.0e300, 1.0e300]
            }),
            1.0e300
        );
    }

    #[test]
    #[should_panic(expected = "the list is empty")]
    fn average_of_nothing_is_red() {
        let _ = average(ReduceIn { list: vec![] });
    }

    proptest::proptest! {
        // Between min and max, and a constant list averages to the constant.
        #[test]
        fn average_property_bounded_by_the_extremes(
            xs in proptest::collection::vec(-1.0e6..1.0e6_f64, 1..40),
            c in -1.0e6..1.0e6_f64,
        ) {
            let mean = average(ReduceIn { list: xs.clone() });
            let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            proptest::prop_assert!(mean >= lo - 1.0e-9 && mean <= hi + 1.0e-9);
            proptest::prop_assert_eq!(average(ReduceIn { list: vec![c; 4] }), c);
        }
    }

    #[test]
    fn average_determinism_golden_hash() {
        // Dyadic inputs with a power-of-two count: the mean is exact.
        assert_eq!(
            hex(average(ReduceIn {
                list: vec![0.5, 0.25, 0.125, 1.0]
            })),
            "d9b0bb57bd66847a9f73e5510a399eef1e5fdaaaeb61784abcadcc1db7baf949"
        );
    }
}
