//! The `bounds` node.

use cicada_core::scalar::Domain;
use cicada_macros::node;

use super::ReduceIn;
/// Bounds — the domain spanned by a list of numbers, from its smallest to
/// its largest.
///
/// # Returns
///
/// The domain `min..max` (a singleton list gives an empty domain
/// `x..x`).
///
/// # Panics
///
/// Panics when the list is empty — nothing has no bounds (loud refusal).
///
/// # Examples
///
/// ```cic
/// xs = [3.0, -1.0, 2.5]
/// span = bounds(list=xs)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Bounds")]
#[must_use]
pub fn bounds(input: ReduceIn) -> Domain {
    assert!(
        !input.list.is_empty(),
        "bounds: the list is empty — nothing has no bounds"
    );
    let start = input.list.iter().copied().fold(f64::INFINITY, f64::min);
    let end = input.list.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Domain::new(start, end)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn bounds_table_cases() {
        assert_eq!(
            bounds(ReduceIn {
                list: vec![3.0, -1.0, 2.5]
            }),
            Domain::new(-1.0, 3.0)
        );
        assert_eq!(
            bounds(ReduceIn { list: vec![7.5] }),
            Domain::new(7.5, 7.5),
            "a singleton spans an empty domain"
        );
        assert_eq!(
            bounds(ReduceIn {
                list: vec![f64::NEG_INFINITY, 0.0, f64::INFINITY]
            }),
            Domain::new(f64::NEG_INFINITY, f64::INFINITY)
        );
    }

    #[test]
    #[should_panic(expected = "the list is empty")]
    fn bounds_of_nothing_is_red() {
        let _ = bounds(ReduceIn { list: vec![] });
    }

    proptest::proptest! {
        // Every element lies inside; both ends are elements; order-independent.
        #[test]
        fn bounds_property_tight_and_order_free(
            xs in proptest::collection::vec(-1.0e6..1.0e6_f64, 1..40),
        ) {
            let span = bounds(ReduceIn { list: xs.clone() });
            proptest::prop_assert!(span.start <= span.end);
            proptest::prop_assert!(xs.iter().all(|&x| span.start <= x && x <= span.end));
            proptest::prop_assert!(xs.contains(&span.start) && xs.contains(&span.end));
            let mut reversed = xs.clone();
            reversed.reverse();
            proptest::prop_assert_eq!(bounds(ReduceIn { list: reversed }), span);
        }
    }

    #[test]
    fn bounds_determinism_golden_hash() {
        assert_eq!(
            hex(bounds(ReduceIn {
                list: vec![3.0, -1.0, 2.5]
            })),
            "f0fb1cf5f1b5fa777976e7e0785abf114bdc9a58d693ddea1fbdb71cb0a0b54a"
        );
    }
}
