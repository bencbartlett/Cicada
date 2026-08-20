//! The `sort` node.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
use cicada_macros::{Ports, node};

/// Inputs for [`sort`].
#[derive(Ports, Clone, Debug)]
pub struct SortIn {
    /// The numeric sort keys — one per slot of `values` (strict zip).
    pub keys: Vec<f64>,
    /// The list to reorder by the keys (to sort numbers alone, wire the
    /// same list to both ports).
    pub values: Vec<ElemSlot>,
}

/// Outputs of [`sort`].
#[derive(Ports, Clone, Debug)]
pub struct SortOut {
    /// The keys in ascending order.
    pub keys: Vec<f64>,
    /// `values` reordered by the keys (a slot rides with its key, absent
    /// slots included).
    pub sorted: Vec<ElemSlot>,
    /// Provenance: `map[i]` is the source index of `sorted[i]`.
    pub map: IndexMap,
}

/// Sort — reorder a list by numeric keys, ascending and stable (equal keys
/// keep their source order), returning the sorted keys, the reordered
/// values, and the index map back into the source.
///
/// # Panics
///
/// Panics when the key count differs from the value count — strict zip,
/// both counts in the message.
///
/// # Examples
///
/// ```cic
/// heights = [2.5, 0.5, 1.5]
/// labels = ["tall", "short", "mid"]
/// ordered_heights, ordered_labels, sources = sort(keys=heights, values=labels)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Sort List")]
#[must_use]
pub fn sort(input: SortIn) -> SortOut {
    assert!(
        input.keys.len() == input.values.len(),
        "sort: {} keys for a list of {} slots — zip is strict",
        input.keys.len(),
        input.values.len()
    );
    // Sort indices, not slots: one stable sort yields every output, and
    // `total_cmp` is a total order (NaN never reaches a node — value
    // construction refuses it; −0.0 is canonicalized to 0.0 there too).
    let mut order: Vec<usize> = (0..input.keys.len()).collect();
    order.sort_by(|&a, &b| input.keys[a].total_cmp(&input.keys[b]));
    SortOut {
        keys: order.iter().map(|&i| input.keys[i]).collect(),
        sorted: order.iter().map(|&i| input.values[i].clone()).collect(),
        map: IndexMap(order.iter().map(|&i| i as u64).collect()),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // keys pass through untouched — exact by contract
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn sort_table_cases() {
        let out = sort(SortIn {
            keys: vec![2.5, 0.5, 1.5],
            values: numbers(&[20.0, 0.0, 10.0]),
        });
        assert_eq!(out.keys, vec![0.5, 1.5, 2.5]);
        assert_eq!(data(&out.sorted[0]), Some(&ValueData::Number(0.0)));
        assert_eq!(data(&out.sorted[1]), Some(&ValueData::Number(10.0)));
        assert_eq!(data(&out.sorted[2]), Some(&ValueData::Number(20.0)));
        assert_eq!(out.map, IndexMap(vec![1, 2, 0]));
        // Stable: equal keys keep source order; a hole rides with its key.
        let ties = sort(SortIn {
            keys: vec![1.0, 0.0, 1.0, 0.0],
            values: slots(&[Some(1.0), None, Some(3.0), Some(4.0)]),
        });
        assert_eq!(ties.map, IndexMap(vec![1, 3, 0, 2]));
        assert!(!ties.sorted[0].is_present(), "the hole rode with key 0.0");
        // Negative and extreme keys order numerically.
        let extremes = sort(SortIn {
            keys: vec![f64::MAX, -1.0e300, 0.0, f64::MIN_POSITIVE],
            values: numbers(&[0.0, 1.0, 2.0, 3.0]),
        });
        assert_eq!(extremes.map, IndexMap(vec![1, 2, 3, 0]));
        // Empty in, empty out.
        let empty = sort(SortIn {
            keys: vec![],
            values: vec![],
        });
        assert!(empty.keys.is_empty() && empty.sorted.is_empty() && empty.map.0.is_empty());
    }

    #[test]
    #[should_panic(expected = "2 keys for a list of 3 slots")]
    fn sort_key_count_mismatch_is_red() {
        let _ = sort(SortIn {
            keys: vec![1.0, 2.0],
            values: numbers(&[1.0, 2.0, 3.0]),
        });
    }

    proptest::proptest! {
        // The map is a permutation; sorted[i] is values[map[i]] and keys[i]
        // is keys[map[i]]; output keys are non-decreasing; ties keep source
        // order (stability).
        #[test]
        fn sort_property_stable_permutation(
            values in holed_list(30),
            raw_keys in proptest::collection::vec(-5i64..5, 30),
        ) {
            let list = slots(&values);
            #[allow(clippy::cast_precision_loss)]
            let keys: Vec<f64> = raw_keys[..values.len()].iter().map(|&k| k as f64).collect();
            let out = sort(SortIn { keys: keys.clone(), values: list.clone() });
            proptest::prop_assert_eq!(out.map.0.len(), list.len());
            let mut seen = vec![false; list.len()];
            for (i, &source) in out.map.0.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let source = source as usize;
                proptest::prop_assert!(!seen[source], "a permutation visits each index once");
                seen[source] = true;
                proptest::prop_assert_eq!(&out.sorted[i], &list[source]);
                proptest::prop_assert_eq!(out.keys[i], keys[source]);
                if i > 0 {
                    proptest::prop_assert!(out.keys[i - 1] <= out.keys[i]);
                    if out.keys[i - 1] == out.keys[i] {
                        proptest::prop_assert!(out.map.0[i - 1] < out.map.0[i], "stable");
                    }
                }
            }
        }
    }

    // Golden hashes of the three sealed outputs.
    #[test]
    fn sort_determinism_golden_hash() {
        let out = sort(SortIn {
            keys: vec![3.0, 1.0, 2.0, 1.0],
            values: slots(&[Some(30.0), None, Some(20.0), Some(10.0)]),
        });
        assert_eq!(
            hex(out.keys),
            "3d9d733088b663f12a31a1ed3e7a4879dbc6b4c9803ced481021a115ff7dbace"
        );
        assert_eq!(
            hex(out.sorted),
            "71fad44c3d88ebbd18a4dd7e020f0cf2d2165b939b10b3f9ebdd82ff18fcad8f"
        );
        assert_eq!(
            hex(out.map),
            "b1cd4e50e9d3c4111b0a699638cc82fa67ef1fc5ffd3d0ac76a17e4686743636"
        );
    }
}
