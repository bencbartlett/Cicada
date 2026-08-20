//! The `split_list` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`split_list`].
#[derive(Ports, Clone, Debug)]
pub struct SplitListIn {
    /// The list to split.
    pub list: Vec<ElemSlot>,
    /// The split index: slots before it go to `a`, the rest to `b`
    /// (`0` puts everything in `b`, the length puts everything in `a`).
    pub index: i64,
}

/// Outputs of [`split_list`].
#[derive(Ports, Clone, Debug)]
pub struct SplitListOut {
    /// The slots before the index, in order.
    pub a: Vec<ElemSlot>,
    /// The slots from the index on, in order.
    pub b: Vec<ElemSlot>,
}

/// Split List — cut a list in two at an index (`[a, b, c]` at `1` →
/// `[a]` and `[b, c]`). Slot-preserving: absent slots land in their half.
/// Both halves keep source order, so no index map is needed (`b[i]` came
/// from `index + i`).
///
/// # Panics
///
/// Panics when `index` is negative or beyond the list's slot count — both
/// in the message (never a silent clamp).
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0, 5.0]
/// head, tail = split_list(list=xs, index=2)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Split List")]
#[must_use]
pub fn split_list(input: SplitListIn) -> SplitListOut {
    let mut a = input.list;
    let index = usize::try_from(input.index)
        .ok()
        .filter(|&index| index <= a.len())
        .unwrap_or_else(|| {
            panic!(
                "split_list: index {} out of range 0..={} (the list has {} slots)",
                input.index,
                a.len(),
                a.len()
            )
        });
    let b = a.split_off(index);
    SplitListOut { a, b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn split_list_table_cases() {
        let out = split_list(SplitListIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
            index: 1,
        });
        assert_eq!(out.a, numbers(&[1.0]));
        assert_eq!(out.b, slots(&[None, Some(3.0)]), "the hole lands in b");
        // The ends: everything in b, everything in a.
        let at_zero = split_list(SplitListIn {
            list: numbers(&[1.0, 2.0]),
            index: 0,
        });
        assert!(at_zero.a.is_empty());
        assert_eq!(at_zero.b, numbers(&[1.0, 2.0]));
        let at_len = split_list(SplitListIn {
            list: numbers(&[1.0, 2.0]),
            index: 2,
        });
        assert_eq!(at_len.a, numbers(&[1.0, 2.0]));
        assert!(at_len.b.is_empty());
        // The empty list splits at 0 into two empties.
        let empty = split_list(SplitListIn {
            list: vec![],
            index: 0,
        });
        assert!(empty.a.is_empty() && empty.b.is_empty());
    }

    #[test]
    #[should_panic(expected = "index 4 out of range 0..=3")]
    fn split_list_index_beyond_length_is_red() {
        let _ = split_list(SplitListIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            index: 4,
        });
    }

    #[test]
    #[should_panic(expected = "index -1 out of range")]
    fn split_list_negative_index_is_red() {
        let _ = split_list(SplitListIn {
            list: numbers(&[1.0]),
            index: -1,
        });
    }

    proptest::proptest! {
        // a ++ b is the source; a has exactly `index` slots.
        #[test]
        fn split_list_property_halves_concatenate_back(
            values in holed_list(30),
            fraction in 0.0..=1.0_f64,
        ) {
            let list = slots(&values);
            #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let index = ((list.len() as f64) * fraction).floor() as usize;
            let out = split_list(SplitListIn {
                list: list.clone(),
                index: i64::try_from(index).expect("small"),
            });
            proptest::prop_assert_eq!(out.a.len(), index);
            let mut joined = out.a;
            joined.extend(out.b);
            proptest::prop_assert_eq!(joined, list);
        }
    }

    // Golden hashes of both sealed halves — holes included.
    #[test]
    fn split_list_determinism_golden_hash() {
        let out = split_list(SplitListIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0)]),
            index: 3,
        });
        assert_eq!(
            hex(out.a),
            "3c03166ac4b61dccf078fa869b66dda7e5fec22449d27ffb7f897a623014baf8"
        );
        assert_eq!(
            hex(out.b),
            "04709fbd6e5d47adfd0e3379fa0b253db23552f3d261aaf4a61cc75944fcc50a"
        );
    }
}
