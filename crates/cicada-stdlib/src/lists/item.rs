//! The `item` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`item`].
#[derive(Ports, Clone, Debug)]
pub struct ItemIn {
    /// The list.
    pub list: Vec<ElemSlot>,
    /// Zero-based index.
    pub index: i64,
    /// Wrap out-of-range indices around the list (modular).
    #[port(default = false)]
    pub wrap: bool,
}

/// List Item — one element of a list by index. Selecting an absent
/// (`Optional`) slot yields an absent element — `E` carries the `?`, so
/// the output is `Point?` exactly when the list is `[Point?]`.
///
/// # Panics
///
/// Panics when the list is empty, or when `index` is out of range and
/// `wrap` is off.
///
/// # Examples
///
/// ```cic
/// xs = [10.0, 20.0, 30.0]
/// last = item(list=xs, index=-1, wrap=True)
/// ```
#[node(category = "List & axis", tier = "S", version = 2, gh = "List Item")]
#[must_use]
pub fn item(input: ItemIn) -> ElemSlot {
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

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, number, numbers, slots};

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
        assert_eq!(data(&get(0, false)), Some(&ValueData::Number(10.0)));
        assert_eq!(data(&get(2, false)), Some(&ValueData::Number(30.0)));
        assert_eq!(data(&get(3, true)), Some(&ValueData::Number(10.0)), "wraps");
        assert_eq!(
            data(&get(-1, true)),
            Some(&ValueData::Number(30.0)),
            "negative wraps from the end"
        );
        // Hole-aware: selecting an absent slot yields an absent element.
        let holed = slots(&[Some(1.0), None]);
        assert!(
            !item(ItemIn {
                list: holed,
                index: 1,
                wrap: false,
            })
            .is_present()
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

    proptest::proptest! {
        // item(wrap=true) is total: any index selects index mod len — and a
        // hole selects as a hole.
        #[test]
        fn item_property_wrap_is_modular(
            values in proptest::collection::vec(
                proptest::option::of(-1.0e6..1.0e6_f64),
                1..20,
            ),
            index in proptest::num::i64::ANY,
        ) {
            let list = slots(&values);
            let got = item(ItemIn { list, index, wrap: true });
            #[allow(
                clippy::cast_possible_wrap,
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation
            )]
            let expected = values[index.rem_euclid(values.len() as i64) as usize];
            proptest::prop_assert_eq!(
                data(&got).cloned(),
                expected.map(ValueData::Number)
            );
        }
    }

    // item passes the SAME sealed value through — hash-identical, the
    // determinism contract for a pass-through node.
    #[test]
    fn item_determinism_passes_hash_through() {
        let element = number(4.25);
        let want = element.0.as_ref().unwrap().hash();
        let got = item(ItemIn {
            list: vec![number(1.0), element],
            index: 1,
            wrap: false,
        });
        assert_eq!(got.0.as_ref().unwrap().hash(), want);
        // Sealing an absent selection gives the canonical Nothing value.
        let absent = item(ItemIn {
            list: slots(&[None]),
            index: 0,
            wrap: false,
        });
        assert_eq!(
            hex(absent),
            hex(ElemSlot(None)),
            "absent output seals to the one Nothing hash"
        );
    }
}
