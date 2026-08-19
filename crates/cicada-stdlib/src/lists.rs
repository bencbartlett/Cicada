//! List & axis nodes (docs/08 §Catalog 4). The wire-level combinators
//! (`each()` map/zip) live in the dialect; these are the node forms the
//! spike needs. Element kinds flow through the `E` type variable — `item`
//! of a `[Point]` is a `Point`, statically.

use cicada_core::marshal::ElemValue;
use cicada_macros::{Ports, node};

/// Inputs for [`item`].
#[derive(Ports, Clone, Debug)]
pub struct ItemIn {
    /// The list.
    pub list: Vec<ElemValue>,
    /// Zero-based index.
    pub index: i64,
    /// Wrap out-of-range indices around the list (modular).
    #[port(default = false)]
    pub wrap: bool,
}

/// List Item — one element of a list by index.
///
/// # Panics
///
/// Panics when the list is empty, or when `index` is out of range and
/// `wrap` is off. Lists with absent (`Optional`) slots refuse at
/// marshalling — hole-aware selection arrives with the Optional-flow
/// nodes (v0.1).
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn item(input: ItemIn) -> ElemValue {
    let len = input.list.len();
    assert!(len > 0, "item: the list is empty");
    #[allow(clippy::cast_possible_wrap)] // list lengths are far below i64::MAX
    let len_i = len as i64;
    let index = if input.wrap {
        input.index.rem_euclid(len_i)
    } else {
        assert!(
            (0..len_i).contains(&input.index),
            "item: index {} out of range 0..{len} (wrap=false)",
            input.index
        );
        input.index
    };
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // 0 <= index < len
    input.list[index as usize].clone()
}

/// Inputs for [`length`].
#[derive(Ports, Clone, Debug)]
pub struct LengthIn {
    /// The list — absent (`Optional`) slots count too.
    pub list: Vec<Option<ElemValue>>,
}

/// List Length — the number of slots in a list (absent slots included —
/// slot-preserving nulls keep their places, docs/08 rule 6).
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn length(input: LengthIn) -> i64 {
    #[allow(clippy::cast_possible_wrap)] // list lengths are far below i64::MAX
    let count = input.list.len() as i64;
    count
}

#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn number(x: f64) -> ElemValue {
        ElemValue(HashedValue::new(ValueData::Number(x)).unwrap())
    }

    fn numbers(values: &[f64]) -> Vec<ElemValue> {
        values.iter().map(|&x| number(x)).collect()
    }

    #[test]
    fn item_table_cases() {
        let list = numbers(&[10.0, 20.0, 30.0]);
        let get = |index, wrap| {
            item(ItemIn {
                list: list.clone(),
                index,
                wrap,
            })
        };
        assert_eq!(*get(0, false).0.data(), ValueData::Number(10.0));
        assert_eq!(*get(2, false).0.data(), ValueData::Number(30.0));
        assert_eq!(*get(3, true).0.data(), ValueData::Number(10.0), "wraps");
        assert_eq!(
            *get(-1, true).0.data(),
            ValueData::Number(30.0),
            "negative wraps from the end"
        );
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn item_out_of_range_is_red() {
        let _ = item(ItemIn {
            list: numbers(&[1.0]),
            index: 1,
            wrap: false,
        });
    }

    #[test]
    #[should_panic(expected = "empty")]
    fn item_empty_list_is_red() {
        let _ = item(ItemIn {
            list: vec![],
            index: 0,
            wrap: true,
        });
    }

    #[test]
    fn length_counts_slots_including_holes() {
        assert_eq!(length(LengthIn { list: vec![] }), 0);
        assert_eq!(
            length(LengthIn {
                list: vec![Some(number(1.0)), None, Some(number(2.0))],
            }),
            3,
            "absent slots keep their places"
        );
    }

    proptest::proptest! {
        // item(wrap=true) is total: any index selects index mod len.
        #[test]
        fn item_property_wrap_is_modular(
            values in proptest::collection::vec(-1.0e6..1.0e6_f64, 1..20),
            index in proptest::num::i64::ANY,
        ) {
            let list = numbers(&values);
            let got = item(ItemIn { list, index, wrap: true });
            #[allow(
                clippy::cast_possible_wrap,
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation
            )]
            let expected = values[index.rem_euclid(values.len() as i64) as usize];
            proptest::prop_assert_eq!(got.0.data(), &ValueData::Number(expected));
        }

        // length counts every slot — absent (None) slots included.
        #[test]
        fn length_property_counts_all_slots(
            slots in proptest::collection::vec(
                proptest::option::of(-1.0e6..1.0e6_f64),
                0..40,
            ),
        ) {
            let list: Vec<Option<ElemValue>> =
                slots.iter().map(|&slot| slot.map(number)).collect();
            let want = i64::try_from(slots.len()).expect("len < 40 fits i64");
            proptest::prop_assert_eq!(length(LengthIn { list }), want);
        }
    }

    // item passes the SAME sealed value through — hash-identical, the
    // determinism contract for a pass-through node.
    #[test]
    fn item_determinism_passes_hash_through() {
        let element = number(4.25);
        let want = element.0.hash();
        let got = item(ItemIn {
            list: vec![number(1.0), element],
            index: 1,
            wrap: false,
        });
        assert_eq!(got.0.hash(), want);
    }

    #[test]
    fn length_determinism_golden_hash() {
        let list: Vec<Option<ElemValue>> = (0..7).map(|i| Some(number(f64::from(i)))).collect();
        let out = length(LengthIn { list });
        let sealed = HashedValue::new(ValueData::Integer(out)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "64ad282ae443e2988c185814d9431a6fda5e3053a917ebfffb9076953f9b2d3a"
        );
    }
}
