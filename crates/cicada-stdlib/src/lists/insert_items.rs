//! The `insert_items` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`insert_items`].
#[derive(Ports, Clone, Debug)]
pub struct InsertItemsIn {
    /// The list to insert into.
    pub list: Vec<ElemSlot>,
    /// The elements to insert, one per index (strict zip with `indices`).
    pub items: Vec<ElemSlot>,
    /// Where each item goes, applied in order: index `i` is a position in
    /// the list AS IT IS after the previous insertions (`0` = the front,
    /// the current length = the back).
    pub indices: Vec<i64>,
}

/// Insert Items — insert elements into a list at the given indices, one
/// insertion after another (`[a, b, c]` with items `[x, y]` at `[1, 3]` →
/// `[a, x, b, y, c]`; GH Insert Items). Each index addresses the list as
/// the previous insertions left it — `[0, 0]` puts the second item in
/// front of the first. Slot-preserving: absent slots insert like any
/// other element. No wrapping: an index past the current length is red,
/// never a silent append.
///
/// # Returns
///
/// The list with every item inserted, `list` order preserved.
///
/// # Panics
///
/// Panics when the item count differs from the index count — strict zip,
/// both counts in the message — or when an index is negative or beyond
/// the list's length at its turn (the index, its position, and that
/// length in the message).
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0]
/// extra = [10.0, 20.0]
/// widened = insert_items(list=xs, items=extra, indices=[1, 3])
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Insert Items")]
#[must_use]
pub fn insert_items(input: InsertItemsIn) -> Vec<ElemSlot> {
    let InsertItemsIn {
        mut list,
        items,
        indices,
    } = input;
    assert!(
        items.len() == indices.len(),
        "insert_items: {} items for {} indices — zip is strict",
        items.len(),
        indices.len()
    );
    for (n, (item, index)) in items.into_iter().zip(indices).enumerate() {
        let at = usize::try_from(index)
            .ok()
            .filter(|&at| at <= list.len())
            .unwrap_or_else(|| {
                panic!(
                    "insert_items: index {index} (item {n}) out of range 0..={} — the list \
                     has {} slots at that turn (insertions apply in order)",
                    list.len(),
                    list.len()
                )
            });
        list.insert(at, item);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, hole, holed_list, number, numbers, slots};

    #[test]
    fn insert_items_table_cases() {
        // The doc example: each index addresses the list as it is then.
        let out = insert_items(InsertItemsIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            items: numbers(&[10.0, 20.0]),
            indices: vec![1, 3],
        });
        assert_eq!(out, numbers(&[1.0, 10.0, 2.0, 20.0, 3.0]));
        // Two inserts at the front: the second lands before the first.
        let front = insert_items(InsertItemsIn {
            list: numbers(&[1.0]),
            items: numbers(&[10.0, 20.0]),
            indices: vec![0, 0],
        });
        assert_eq!(front.as_slice(), numbers(&[20.0, 10.0, 1.0]).as_slice());
        // The current length appends; into an empty list, twice.
        let appended = insert_items(InsertItemsIn {
            list: vec![],
            items: numbers(&[10.0, 20.0]),
            indices: vec![0, 1],
        });
        assert_eq!(appended, numbers(&[10.0, 20.0]));
        // Holes insert and are inserted around like any slot.
        let holey = insert_items(InsertItemsIn {
            list: slots(&[Some(1.0), None]),
            items: vec![hole(), number(5.0)],
            indices: vec![2, 0],
        });
        assert_eq!(holey, slots(&[Some(5.0), Some(1.0), None, None]));
        // Nothing to insert: the identity.
        let same = insert_items(InsertItemsIn {
            list: numbers(&[1.0, 2.0]),
            items: vec![],
            indices: vec![],
        });
        assert_eq!(same, numbers(&[1.0, 2.0]));
    }

    #[test]
    #[should_panic(expected = "2 items for 1 indices — zip is strict")]
    fn insert_items_count_mismatch_is_red() {
        let _ = insert_items(InsertItemsIn {
            list: numbers(&[1.0]),
            items: numbers(&[10.0, 20.0]),
            indices: vec![0],
        });
    }

    #[test]
    #[should_panic(expected = "index 3 (item 1) out of range 0..=2")]
    fn insert_items_index_beyond_the_current_length_is_red() {
        // The first insert is fine (0..=1); the second asks 3 of a 2-list.
        let _ = insert_items(InsertItemsIn {
            list: numbers(&[1.0]),
            items: numbers(&[10.0, 20.0]),
            indices: vec![1, 3],
        });
    }

    #[test]
    #[should_panic(expected = "index -1 (item 0) out of range")]
    fn insert_items_negative_index_is_red() {
        let _ = insert_items(InsertItemsIn {
            list: numbers(&[1.0]),
            items: numbers(&[10.0]),
            indices: vec![-1],
        });
    }

    proptest::proptest! {
        // Every in-range insertion sequence keeps the source slots in
        // source order and adds exactly the items (tagged by a disjoint
        // value range so the two can be told apart).
        #[test]
        fn insert_items_property_source_order_survives(
            values in holed_list(20),
            fractions in proptest::collection::vec(0.0..=1.0_f64, 0..8),
        ) {
            let list = slots(&values);
            #[allow(clippy::cast_precision_loss)]
            let items: Vec<ElemSlot> = (0..fractions.len())
                .map(|i| number(1.0e9 + i as f64))
                .collect();
            // The list grows by one per insertion: index n draws from
            // 0..=len + n, always in range.
            let mut indices = Vec::new();
            for (n, fraction) in fractions.iter().enumerate() {
                let length = list.len() + n;
                #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let at = ((length as f64) * fraction).floor() as usize;
                indices.push(i64::try_from(at).expect("small"));
            }
            let out = insert_items(InsertItemsIn {
                list: list.clone(),
                items: items.clone(),
                indices,
            });
            proptest::prop_assert_eq!(out.len(), list.len() + items.len());
            let is_item = |slot: &ElemSlot| items.contains(slot);
            let kept: Vec<ElemSlot> = out.iter().filter(|s| !is_item(s)).cloned().collect();
            proptest::prop_assert_eq!(kept, list);
            let inserted = out.iter().filter(|s| is_item(s)).count();
            proptest::prop_assert_eq!(inserted, items.len());
        }
    }

    // Golden hash of the sealed output — a hole inserted among numbers.
    #[test]
    fn insert_items_determinism_golden_hash() {
        let out = insert_items(InsertItemsIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            items: vec![number(10.0), hole()],
            indices: vec![1, 4],
        });
        assert_eq!(
            hex(out),
            "3cfe33f11a6fd59c209aae4089b8d1ff748867fbd7e1e4756f2ad4bbfbe0bda0"
        );
    }
}
