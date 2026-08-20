//! The `group_by` node.

use std::collections::HashMap;

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`group_by`].
#[derive(Ports, Clone, Debug)]
pub struct GroupByIn {
    /// The numeric group key of each slot — one per slot of `values`
    /// (strict zip); integers widen, so plate numbers and indices work.
    pub keys: Vec<f64>,
    /// The list to group.
    pub values: Vec<ElemSlot>,
}

/// Outputs of [`group_by`].
#[derive(Ports, Clone, Debug)]
pub struct GroupByOut {
    /// One group per distinct key, in order of the key's first occurrence;
    /// each group holds its slots in source order.
    pub groups: Vec<Vec<ElemSlot>>,
    /// The distinct keys, index-aligned with `groups`.
    pub keys: Vec<f64>,
}

/// Group By — gather a list into groups by numeric key (the honest version
/// of most Path Mapper recipes, docs/09): one group per distinct key, in
/// order of first occurrence, source order kept inside each group, absent
/// slots grouped like any other. Keys compare exactly. For provenance,
/// group the indices themselves — `group_by(keys=k, values=series(…))` —
/// or sort by the same keys (`sort` returns the `IndexMap`).
///
/// # Panics
///
/// Panics when the key count differs from the value count — strict zip,
/// both counts in the message.
///
/// # Examples
///
/// ```cic
/// plate = [2, 1, 2, 1, 3]
/// parts = [10.0, 20.0, 30.0, 40.0, 50.0]
/// per_plate, plates = group_by(keys=plate, values=parts)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = none)]
#[must_use]
pub fn group_by(input: GroupByIn) -> GroupByOut {
    assert!(
        input.keys.len() == input.values.len(),
        "group_by: {} keys for a list of {} slots — zip is strict",
        input.keys.len(),
        input.values.len()
    );
    // First-occurrence order lives in the Vecs; the map is only the lookup
    // (keyed by bit pattern: values are canonical — no NaN, no −0.0).
    let mut index_of: HashMap<u64, usize> = HashMap::new();
    let mut groups: Vec<Vec<ElemSlot>> = Vec::new();
    let mut keys: Vec<f64> = Vec::new();
    for (key, slot) in input.keys.into_iter().zip(input.values) {
        let group = *index_of.entry(key.to_bits()).or_insert_with(|| {
            groups.push(Vec::new());
            keys.push(key);
            groups.len() - 1
        });
        groups[group].push(slot);
    }
    GroupByOut { groups, keys }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // keys pass through untouched — exact by contract
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn group_by_table_cases() {
        let out = group_by(GroupByIn {
            keys: vec![2.0, 1.0, 2.0, 1.0, 3.0],
            values: slots(&[Some(10.0), None, Some(30.0), Some(40.0), Some(50.0)]),
        });
        assert_eq!(out.keys, vec![2.0, 1.0, 3.0], "first-occurrence order");
        assert_eq!(out.groups.len(), 3);
        assert_eq!(data(&out.groups[0][0]), Some(&ValueData::Number(10.0)));
        assert_eq!(data(&out.groups[0][1]), Some(&ValueData::Number(30.0)));
        assert!(!out.groups[1][0].is_present(), "the hole grouped under 1.0");
        assert_eq!(data(&out.groups[1][1]), Some(&ValueData::Number(40.0)));
        assert_eq!(data(&out.groups[2][0]), Some(&ValueData::Number(50.0)));
        // One key: one group holding everything. Empty in, empty out.
        let one = group_by(GroupByIn {
            keys: vec![-7.5, -7.5],
            values: numbers(&[1.0, 2.0]),
        });
        assert_eq!(one.keys, vec![-7.5]);
        assert_eq!(one.groups, vec![numbers(&[1.0, 2.0])]);
        let empty = group_by(GroupByIn {
            keys: vec![],
            values: vec![],
        });
        assert!(empty.groups.is_empty() && empty.keys.is_empty());
    }

    #[test]
    #[should_panic(expected = "2 keys for a list of 3 slots")]
    fn group_by_key_count_mismatch_is_red() {
        let _ = group_by(GroupByIn {
            keys: vec![1.0, 2.0],
            values: numbers(&[1.0, 2.0, 3.0]),
        });
    }

    proptest::proptest! {
        // Groups partition the list: sizes sum to n, every slot lands in the
        // group of its key in source order, keys are distinct and appear in
        // first-occurrence order.
        #[test]
        fn group_by_property_partitions_by_key(
            values in holed_list(30),
            raw_keys in proptest::collection::vec(-3i64..3, 30),
        ) {
            let list = slots(&values);
            #[allow(clippy::cast_precision_loss)]
            let keys: Vec<f64> = raw_keys[..values.len()].iter().map(|&k| k as f64).collect();
            let out = group_by(GroupByIn { keys: keys.clone(), values: list.clone() });
            proptest::prop_assert_eq!(out.groups.len(), out.keys.len());
            let total: usize = out.groups.iter().map(Vec::len).sum();
            proptest::prop_assert_eq!(total, list.len());
            for (g, key) in out.keys.iter().enumerate() {
                proptest::prop_assert!(
                    out.keys[..g].iter().all(|k| k != key),
                    "distinct keys"
                );
                let expected: Vec<&ElemSlot> = list
                    .iter()
                    .zip(&keys)
                    .filter(|(_, k)| *k == key)
                    .map(|(slot, _)| slot)
                    .collect();
                proptest::prop_assert_eq!(out.groups[g].iter().collect::<Vec<_>>(), expected);
            }
            // First-occurrence order: each key's first source index increases.
            let firsts: Vec<usize> = out
                .keys
                .iter()
                .map(|key| keys.iter().position(|k| k == key).expect("key came from the input"))
                .collect();
            proptest::prop_assert!(firsts.windows(2).all(|w| w[0] < w[1]));
        }
    }

    // Golden hashes of both sealed outputs — the nested list, holes included.
    #[test]
    fn group_by_determinism_golden_hash() {
        let out = group_by(GroupByIn {
            keys: vec![2.0, 1.0, 2.0, 1.0, 3.0],
            values: slots(&[Some(10.0), None, Some(30.0), Some(40.0), Some(50.0)]),
        });
        assert_eq!(
            hex(out.groups),
            "55cdd2a48d9c435ea2187ebf7123a883ffa008838513b69297dd0de6a8c0b257"
        );
        assert_eq!(
            hex(out.keys),
            "bbee3d3cd0e3eccd790fbdf1f42dded1d8a3b04d2f18194a7b422c6825529160"
        );
    }
}
