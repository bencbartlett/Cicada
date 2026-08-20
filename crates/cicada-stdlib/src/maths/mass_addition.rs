//! The `mass_addition` node.

use cicada_macros::{Ports, node};

use super::ReduceIn;
/// Outputs of [`mass_addition`].
#[derive(Ports, Clone, Debug)]
pub struct MassAdditionOut {
    /// The sum of the list (`0.0` for an empty list).
    pub result: f64,
    /// The running sums: `partial[i]` is the sum of the first `i + 1`
    /// numbers.
    pub partial: Vec<f64>,
}

/// Mass Addition — the sum of a list of numbers and its running sums,
/// accumulated left to right (the order is part of the contract — floating
/// sums depend on it).
///
/// # Examples
///
/// ```cic
/// xs = [0.5, 0.25, 0.125]
/// total, running = mass_addition(list=xs)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Mass Addition"
)]
#[must_use]
pub fn mass_addition(input: ReduceIn) -> MassAdditionOut {
    let mut sum = 0.0;
    let partial = input
        .list
        .iter()
        .map(|x| {
            sum += x;
            sum
        })
        .collect();
    MassAdditionOut {
        result: sum,
        partial,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn mass_addition_table_cases() {
        let out = mass_addition(ReduceIn {
            list: vec![0.5, 0.25, 0.125],
        });
        assert_eq!(out.result, 0.875);
        assert_eq!(out.partial, vec![0.5, 0.75, 0.875]);
        let mixed = mass_addition(ReduceIn {
            list: vec![-1.5, 1.5, 2.0],
        });
        assert_eq!(mixed.result, 2.0);
        assert_eq!(mixed.partial, vec![-1.5, 0.0, 2.0]);
        // The empty sum is zero with no partials; a singleton is itself.
        let empty = mass_addition(ReduceIn { list: vec![] });
        assert_eq!(empty.result, 0.0);
        assert!(empty.partial.is_empty());
        let one = mass_addition(ReduceIn { list: vec![7.0] });
        assert_eq!((one.result, one.partial), (7.0, vec![7.0]));
    }

    proptest::proptest! {
        // As many partials as inputs; the last partial IS the result; each
        // partial is exactly the previous plus the next number (left fold).
        #[test]
        fn mass_addition_property_left_fold(
            xs in proptest::collection::vec(-1.0e6..1.0e6_f64, 0..40),
        ) {
            let out = mass_addition(ReduceIn { list: xs.clone() });
            proptest::prop_assert_eq!(out.partial.len(), xs.len());
            let mut acc = 0.0;
            for (x, p) in xs.iter().zip(&out.partial) {
                acc += x;
                proptest::prop_assert_eq!(*p, acc);
            }
            proptest::prop_assert_eq!(out.result, acc);
        }
    }

    #[test]
    fn mass_addition_determinism_golden_hash() {
        // Dyadic inputs: every partial sum is exact, the hashes platform-free.
        let out = mass_addition(ReduceIn {
            list: vec![0.5, 0.25, 0.125, -1.0],
        });
        assert_eq!(
            hex(out.result),
            "d271aa22bbb0fc363b19de77d771b893ca0a737c5d2e339c7576a0c9515fd20f"
        );
        assert_eq!(
            hex(out.partial),
            "6eb592425f25575f0217529c88db1bdf841eec61d47840a2dc8def7143c1c80e"
        );
    }
}
