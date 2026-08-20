//! The `compact` node.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
use cicada_macros::{Ports, node};

/// Inputs for [`compact`].
#[derive(Ports, Clone, Debug)]
pub struct CompactIn {
    /// The list whose absent slots are to be dropped.
    pub list: Vec<Option<ElemSlot>>,
}

/// Outputs of [`compact`].
#[derive(Ports, Clone, Debug)]
pub struct CompactOut {
    /// The present elements, in order — no absent slots remain.
    pub values: Vec<ElemSlot>,
    /// Provenance: `map[i]` is the source index of `values[i]` (docs/08
    /// rule 6 — identity survives the removal).
    pub map: IndexMap,
}

/// Compact — drop the absent slots of a list, returning the present
/// elements and the index map back into the source (the Optional-flow
/// counterpart of `cull`: the only other way slots leave a list, docs/09).
///
/// The `list` port is `[E?]`: it takes the holes itself, so `E` names the
/// present kind and `values` types `[Point]` for a `[Point?]` in — the
/// checker's "`compact` removes the holes" advice is satisfiable by
/// construction (the `E?`-port rule in the checker's `bind_var`).
///
/// # Examples
///
/// ```cic
/// xs = [3.0, 1.0, 2.0]
/// present, sources = compact(list=xs)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Clean Tree")]
#[must_use]
pub fn compact(input: CompactIn) -> CompactOut {
    let mut values = Vec::new();
    let mut map = Vec::new();
    for (index, slot) in input.list.into_iter().enumerate() {
        // Both spellings of absent fold to a hole: a `None` slot, and a
        // sealed `Nothing` element (`ElemSlot(None)`).
        if let Some(slot @ ElemSlot(Some(_))) = slot {
            values.push(slot);
            map.push(index as u64);
        }
    }
    CompactOut {
        values,
        map: IndexMap(map),
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, hole, holed_list, number, slots};

    /// The marshalled shape of a holed list: a hole is `None`, a present
    /// element is `Some(present slot)`.
    fn optional_slots(values: &[Option<f64>]) -> Vec<Option<ElemSlot>> {
        values
            .iter()
            .map(|v| v.map(|x| ElemSlot(number(x).0)))
            .collect()
    }

    #[test]
    fn compact_table_cases() {
        let out = compact(CompactIn {
            list: optional_slots(&[None, Some(10.0), None, Some(30.0), Some(40.0), None]),
        });
        assert_eq!(out.values.len(), 3);
        assert_eq!(data(&out.values[0]), Some(&ValueData::Number(10.0)));
        assert_eq!(data(&out.values[1]), Some(&ValueData::Number(30.0)));
        assert_eq!(data(&out.values[2]), Some(&ValueData::Number(40.0)));
        assert!(out.values.iter().all(ElemSlot::is_present));
        assert_eq!(out.map, IndexMap(vec![1, 3, 4]));
        // A sealed-Nothing element is the other spelling of absent.
        let sealed = compact(CompactIn {
            list: vec![Some(hole()), Some(number(1.0))],
        });
        assert_eq!(sealed.values.len(), 1);
        assert_eq!(sealed.map, IndexMap(vec![1]));
        // No holes: the identity, with the identity map.
        let full = compact(CompactIn {
            list: optional_slots(&[Some(1.0), Some(2.0)]),
        });
        assert_eq!(full.values, slots(&[Some(1.0), Some(2.0)]));
        assert_eq!(full.map, IndexMap(vec![0, 1]));
        // All holes, and the empty list: nothing kept, empty map.
        let none = compact(CompactIn {
            list: optional_slots(&[None, None]),
        });
        assert!(none.values.is_empty() && none.map.0.is_empty());
        let empty = compact(CompactIn { list: vec![] });
        assert!(empty.values.is_empty() && empty.map.0.is_empty());
    }

    proptest::proptest! {
        // compact: as many values as present slots; values[i] is exactly
        // list[map[i]]; the map is strictly increasing (order preserved);
        // no hole survives.
        #[test]
        fn compact_property_map_is_provenance(values in holed_list(30)) {
            let out = compact(CompactIn { list: optional_slots(&values) });
            let want = values.iter().filter(|v| v.is_some()).count();
            proptest::prop_assert_eq!(out.values.len(), want);
            proptest::prop_assert_eq!(out.map.0.len(), want);
            for (i, &source) in out.map.0.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let source = source as usize;
                proptest::prop_assert!(out.values[i].is_present());
                proptest::prop_assert_eq!(
                    data(&out.values[i]).cloned(),
                    values[source].map(ValueData::Number)
                );
                if i > 0 {
                    proptest::prop_assert!(out.map.0[i - 1] < out.map.0[i]);
                }
            }
        }
    }

    // Golden hashes of both sealed outputs.
    #[test]
    fn compact_determinism_golden_hash() {
        let out = compact(CompactIn {
            list: optional_slots(&[Some(1.0), None, Some(3.0), None, Some(5.0)]),
        });
        assert_eq!(
            hex(out.values),
            "a33f3f379a156a5826e4e7842d0ac4ad25abd5617130a4fd178a2e617c29e0e4"
        );
        assert_eq!(
            hex(out.map),
            "e7dd1907b60f6afebe0e81f43d6d1371be055320b6660a3c36ffd62a1933742e"
        );
    }
}
