//! The `reverse` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`reverse`].
#[derive(Ports, Clone, Debug)]
pub struct ReverseIn {
    /// The list to reverse.
    pub list: Vec<ElemSlot>,
}

/// Reverse — the list in reverse order. Slot-preserving: absent slots
/// reverse with the rest.
///
/// # Returns
///
/// The same slots, last first.
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0]
/// backwards = reverse(list=xs)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Reverse List")]
#[must_use]
pub fn reverse(input: ReverseIn) -> Vec<ElemSlot> {
    let mut list = input.list;
    list.reverse();
    list
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn reverse_table_cases() {
        let out = reverse(ReverseIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
        });
        assert_eq!(data(&out[0]), Some(&ValueData::Number(3.0)));
        assert!(!out[1].is_present(), "the hole reverses with the rest");
        assert_eq!(data(&out[2]), Some(&ValueData::Number(1.0)));
        assert!(reverse(ReverseIn { list: vec![] }).is_empty());
        assert_eq!(
            reverse(ReverseIn {
                list: numbers(&[7.0]),
            }),
            numbers(&[7.0]),
            "a singleton is its own reverse"
        );
    }

    proptest::proptest! {
        // An involution that keeps the length: reverse ∘ reverse = id, and
        // out[i] == in[n - 1 - i].
        #[test]
        fn reverse_property_is_an_involution(values in holed_list(30)) {
            let list = slots(&values);
            let once = reverse(ReverseIn { list: list.clone() });
            proptest::prop_assert_eq!(once.len(), list.len());
            for (i, slot) in once.iter().enumerate() {
                proptest::prop_assert_eq!(slot, &list[list.len() - 1 - i]);
            }
            let twice = reverse(ReverseIn { list: once });
            proptest::prop_assert_eq!(twice, list);
        }
    }

    // Golden hash of the sealed output — holes included.
    #[test]
    fn reverse_determinism_golden_hash() {
        let out = reverse(ReverseIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0)]),
        });
        assert_eq!(
            hex(out),
            "3fd56c7eb8435e7073c051d7fb482532546ef4800ba4683f69d97bd82da84045"
        );
    }
}
