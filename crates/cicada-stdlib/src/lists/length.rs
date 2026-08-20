//! The `length` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`length`].
#[derive(Ports, Clone, Debug)]
pub struct LengthIn {
    /// The list — absent (`Optional`) slots count too.
    pub list: Vec<ElemSlot>,
}

/// List Length — the number of slots in a list (absent slots included —
/// slot-preserving nulls keep their places, docs/08 rule 6).
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0]
/// count = length(list=xs)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "List Length")]
#[must_use]
pub fn length(input: LengthIn) -> i64 {
    #[allow(clippy::cast_possible_wrap)] // list lengths are far below i64::MAX
    let count = input.list.len() as i64;
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, holed_list, number, slots};

    #[test]
    fn length_counts_slots_including_holes() {
        assert_eq!(length(LengthIn { list: vec![] }), 0);
        assert_eq!(
            length(LengthIn {
                list: slots(&[Some(1.0), None, Some(2.0)]),
            }),
            3,
            "absent slots keep their places"
        );
    }

    proptest::proptest! {
        // length counts every slot — absent slots included.
        #[test]
        fn length_property_counts_all_slots(values in holed_list(40)) {
            let want = i64::try_from(values.len()).expect("len < 40 fits i64");
            proptest::prop_assert_eq!(length(LengthIn { list: slots(&values) }), want);
        }
    }

    #[test]
    fn length_determinism_golden_hash() {
        let list: Vec<ElemSlot> = (0..7).map(|i| number(f64::from(i))).collect();
        let out = length(LengthIn { list });
        assert_eq!(
            hex(out),
            "64ad282ae443e2988c185814d9431a6fda5e3053a917ebfffb9076953f9b2d3a"
        );
    }
}
