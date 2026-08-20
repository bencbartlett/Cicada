//! The `concat` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`concat`].
#[derive(Ports, Clone, Debug)]
pub struct ConcatIn {
    /// The leading list.
    pub a: Vec<ElemSlot>,
    /// The trailing list.
    pub b: Vec<ElemSlot>,
}

/// Concat — `a` then `b`, one list (GH Merge for two lists). Slot-preserving:
/// absent slots of either input keep their places in the output.
///
/// # Examples
///
/// ```cic
/// head = [1.0, 2.0]
/// tail = [3.0]
/// joined = concat(a=head, b=tail)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Merge")]
#[must_use]
pub fn concat(input: ConcatIn) -> Vec<ElemSlot> {
    let mut out = input.a;
    out.extend(input.b);
    out
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn concat_table_cases() {
        let out = concat(ConcatIn {
            a: slots(&[Some(1.0), None]),
            b: slots(&[None, Some(4.0)]),
        });
        assert_eq!(out.len(), 4);
        assert_eq!(data(&out[0]), Some(&ValueData::Number(1.0)));
        assert!(!out[1].is_present() && !out[2].is_present());
        assert_eq!(data(&out[3]), Some(&ValueData::Number(4.0)));
        assert!(
            concat(ConcatIn {
                a: vec![],
                b: vec![]
            })
            .is_empty()
        );
        assert_eq!(
            concat(ConcatIn {
                a: vec![],
                b: numbers(&[7.0]),
            })
            .len(),
            1
        );
    }

    proptest::proptest! {
        // concat: length adds, `a` is the prefix and `b` the suffix, slot
        // for slot.
        #[test]
        fn concat_property_prefix_suffix(a in holed_list(20), b in holed_list(20)) {
            let (a, b) = (slots(&a), slots(&b));
            let out = concat(ConcatIn { a: a.clone(), b: b.clone() });
            proptest::prop_assert_eq!(out.len(), a.len() + b.len());
            proptest::prop_assert_eq!(&out[..a.len()], &a[..]);
            proptest::prop_assert_eq!(&out[a.len()..], &b[..]);
        }
    }

    // Golden hash of the sealed output — holes included, since the hole
    // layout is part of the value.
    #[test]
    fn concat_determinism_golden_hash() {
        let source = slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]);
        let joined = concat(ConcatIn {
            a: source.clone(),
            b: numbers(&[6.0]),
        });
        assert_eq!(
            hex(joined),
            "a5cab35bd52b2e140e6fb6bd4a6f96c128a532c69475fb306fff47d6d7397437"
        );
    }
}
