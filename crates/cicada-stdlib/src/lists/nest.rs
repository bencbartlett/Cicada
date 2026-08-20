//! The `nest` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`nest`].
#[derive(Ports, Clone, Debug)]
pub struct NestIn {
    /// The list whose elements become singleton sublists.
    pub list: Vec<ElemSlot>,
}

/// Nest — one nesting level added: each element becomes its own singleton
/// sublist (`[a, b]` → `[[a], [b]]`; GH Graft, docs/09). Slot-preserving:
/// an absent slot becomes a singleton holding an absent slot. `flatten`
/// undoes it.
///
/// # Returns
///
/// One singleton sublist per slot, in order.
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0]
/// singletons = nest(list=xs)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Graft Tree")]
#[must_use]
pub fn nest(input: NestIn) -> Vec<Vec<ElemSlot>> {
    input.list.into_iter().map(|slot| vec![slot]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::flatten::{FlattenIn, flatten};
    use crate::lists::support::{hex, hole, holed_list, number, numbers, slots};

    #[test]
    fn nest_table_cases() {
        assert_eq!(
            nest(NestIn {
                list: slots(&[Some(1.0), None]),
            }),
            vec![vec![number(1.0)], vec![hole()]],
            "a hole nests into a singleton holding a hole"
        );
        assert!(nest(NestIn { list: vec![] }).is_empty());
        assert_eq!(
            nest(NestIn {
                list: numbers(&[7.0]),
            }),
            vec![numbers(&[7.0])]
        );
    }

    proptest::proptest! {
        // As many singletons as slots, each holding exactly its slot; flatten
        // is the left inverse.
        #[test]
        fn nest_property_singletons_flatten_back(values in holed_list(30)) {
            let list = slots(&values);
            let nested = nest(NestIn { list: list.clone() });
            proptest::prop_assert_eq!(nested.len(), list.len());
            for (group, slot) in nested.iter().zip(&list) {
                proptest::prop_assert_eq!(group, &vec![slot.clone()]);
            }
            proptest::prop_assert_eq!(flatten(FlattenIn { list: nested }), list);
        }
    }

    // Golden hash of the sealed nested list — holes included.
    #[test]
    fn nest_determinism_golden_hash() {
        let out = nest(NestIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
        });
        assert_eq!(
            hex(out),
            "0cb2b9ada74488929d40def0b387e0685a4a2d6cf7edf436d639ec3140827637"
        );
    }
}
