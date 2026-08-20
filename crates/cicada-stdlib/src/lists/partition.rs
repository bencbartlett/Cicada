//! The `partition` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`partition`].
#[derive(Ports, Clone, Debug)]
pub struct PartitionIn {
    /// The list to split.
    pub list: Vec<ElemSlot>,
    /// Consecutive group sizes; they must sum to the list's slot count.
    pub sizes: Vec<i64>,
}

/// Partition — consecutive groups of the given sizes (`[a, b, c, d]` with
/// sizes `[1, 3]` → `[[a], [b, c, d]]`). Slot-preserving: absent slots land
/// in their group as absent slots.
///
/// # Panics
///
/// Panics when a size is negative (the offending index and value in the
/// message) or when the sizes do not sum to the list's slot count (both
/// counts in the message) — never a silent short or padded last group.
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn partition(input: PartitionIn) -> Vec<Vec<ElemSlot>> {
    let mut sizes = Vec::with_capacity(input.sizes.len());
    let mut total: usize = 0;
    for (position, &size) in input.sizes.iter().enumerate() {
        let size = usize::try_from(size).unwrap_or_else(|_| {
            panic!("partition: size {position} is negative ({size}) — sizes are slot counts")
        });
        total = total.checked_add(size).unwrap_or_else(|| {
            panic!("partition: sizes overflow (sum exceeds usize::MAX at size {position})")
        });
        sizes.push(size);
    }
    let len = input.list.len();
    assert!(
        total == len,
        "partition: sizes sum to {total} but the list has {len} slots — \
         groups must cover the list exactly"
    );
    // Validated above: the sizes cover the list exactly, so `take` never
    // runs out and nothing is left over.
    let mut rest = input.list.into_iter();
    sizes
        .into_iter()
        .map(|size| rest.by_ref().take(size).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::flatten::{FlattenIn, flatten};
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn partition_table_cases() {
        let groups = partition(PartitionIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0)]),
            sizes: vec![1, 0, 3],
        });
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].len(), 1);
        assert!(groups[1].is_empty(), "zero-sized groups are legal");
        assert_eq!(groups[2].len(), 3);
        assert!(!groups[2][0].is_present(), "the hole lands in its group");
        assert_eq!(data(&groups[2][2]), Some(&ValueData::Number(4.0)));
        // Empty list, no sizes: no groups.
        assert!(
            partition(PartitionIn {
                list: vec![],
                sizes: vec![]
            })
            .is_empty()
        );
    }

    #[test]
    #[should_panic(expected = "sizes sum to 2 but the list has 3 slots")]
    fn partition_short_sizes_are_red() {
        let _ = partition(PartitionIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            sizes: vec![1, 1],
        });
    }

    #[test]
    #[should_panic(expected = "sizes sum to 4 but the list has 3 slots")]
    fn partition_long_sizes_are_red() {
        let _ = partition(PartitionIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            sizes: vec![4],
        });
    }

    #[test]
    #[should_panic(expected = "size 1 is negative (-2)")]
    fn partition_negative_size_is_red() {
        let _ = partition(PartitionIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            sizes: vec![5, -2],
        });
    }

    proptest::proptest! {
        // partition then flatten is the identity, and group lengths are the
        // sizes — for ANY non-negative split of the length.
        #[test]
        fn partition_flatten_property_roundtrip(
            values in holed_list(40),
            cuts in proptest::collection::vec(0usize..=40, 0..6),
        ) {
            let list = slots(&values);
            // Sorted cut points inside the list → sizes summing to len.
            let len = values.len();
            let mut points: Vec<usize> = cuts.into_iter().map(|c| c.min(len)).collect();
            points.sort_unstable();
            let mut sizes = Vec::new();
            let mut previous = 0;
            for point in points {
                sizes.push(i64::try_from(point - previous).expect("small"));
                previous = point;
            }
            sizes.push(i64::try_from(len - previous).expect("small"));
            let groups = partition(PartitionIn { list: list.clone(), sizes: sizes.clone() });
            proptest::prop_assert_eq!(groups.len(), sizes.len());
            for (group, size) in groups.iter().zip(&sizes) {
                proptest::prop_assert_eq!(i64::try_from(group.len()).expect("small"), *size);
            }
            proptest::prop_assert_eq!(flatten(FlattenIn { list: groups }), list);
        }
    }

    // Golden hash of the sealed output — holes included, since the hole
    // layout is part of the value.
    #[test]
    fn partition_determinism_golden_hash() {
        let source = slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]);
        let parts = partition(PartitionIn {
            list: source.clone(),
            sizes: vec![2, 0, 3],
        });
        assert_eq!(
            hex(parts),
            "68b633aabc1c7c343da607d71869085d4a9091cc5fa787d1f629fd38ed007e3c"
        );
    }
}
