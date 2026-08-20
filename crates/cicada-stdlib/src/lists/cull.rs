//! The `cull` node.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
use cicada_macros::{Ports, node};

/// Inputs for [`cull`].
#[derive(Ports, Clone, Debug)]
pub struct CullIn {
    /// The list to filter.
    pub list: Vec<ElemSlot>,
    /// Keep the slot where true, drop it where false — one entry per slot
    /// (strict zip; no pattern repetition).
    pub pattern: Vec<bool>,
}

/// Outputs of [`cull`].
#[derive(Ports, Clone, Debug)]
pub struct CullOut {
    /// The kept slots, in order (absent slots kept as absent when their
    /// pattern entry is true).
    pub kept: Vec<ElemSlot>,
    /// Provenance: `map[i]` is the source index of `kept[i]` (docs/08 rule 6).
    pub map: IndexMap,
}

/// Cull — keep the slots where the pattern is true and drop the rest,
/// returning the kept list and the index map back into the source (the
/// sanctioned way elements leave a list — identity survives).
///
/// # Panics
///
/// Panics when the pattern's length differs from the list's slot count —
/// strict zip, both counts in the message (GH's repeating Cull Pattern is
/// the silent-mismatch behavior docs/09 retires).
///
/// # Examples
///
/// ```cic
/// xs = [10.0, 20.0, 30.0, 40.0]
/// keep = [True, False, True, True]
/// kept, sources = cull(list=xs, pattern=keep)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Cull Pattern")]
#[must_use]
pub fn cull(input: CullIn) -> CullOut {
    assert!(
        input.pattern.len() == input.list.len(),
        "cull: pattern has {} entries for a list of {} slots — zip is strict \
         (pad_last / repeat / truncate are the opt-in adapters)",
        input.pattern.len(),
        input.list.len()
    );
    let mut kept = Vec::new();
    let mut map = Vec::new();
    for (index, (slot, keep)) in input.list.into_iter().zip(input.pattern).enumerate() {
        if keep {
            kept.push(slot);
            map.push(index as u64);
        }
    }
    CullOut {
        kept,
        map: IndexMap(map),
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn cull_table_cases() {
        let out = cull(CullIn {
            list: slots(&[Some(10.0), None, Some(30.0), Some(40.0)]),
            pattern: vec![true, true, false, true],
        });
        assert_eq!(out.kept.len(), 3);
        assert_eq!(data(&out.kept[0]), Some(&ValueData::Number(10.0)));
        assert!(!out.kept[1].is_present(), "a kept hole stays a hole");
        assert_eq!(data(&out.kept[2]), Some(&ValueData::Number(40.0)));
        assert_eq!(out.map, IndexMap(vec![0, 1, 3]));
        // All false: nothing kept, empty map. Empty in, empty out.
        let none = cull(CullIn {
            list: numbers(&[1.0, 2.0]),
            pattern: vec![false, false],
        });
        assert!(none.kept.is_empty() && none.map.0.is_empty());
        let empty = cull(CullIn {
            list: vec![],
            pattern: vec![],
        });
        assert!(empty.kept.is_empty() && empty.map.0.is_empty());
    }

    #[test]
    #[should_panic(expected = "pattern has 2 entries for a list of 3 slots")]
    fn cull_pattern_length_mismatch_is_red() {
        let _ = cull(CullIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            pattern: vec![true, false],
        });
    }

    proptest::proptest! {
        // cull: kept count = true count; kept[i] is exactly list[map[i]];
        // the map is strictly increasing (order preserved).
        #[test]
        fn cull_property_map_is_provenance(
            values in holed_list(30),
            seed in proptest::collection::vec(proptest::bool::ANY, 30),
        ) {
            let list = slots(&values);
            let pattern: Vec<bool> = seed[..values.len()].to_vec();
            let out = cull(CullIn { list: list.clone(), pattern: pattern.clone() });
            let want = pattern.iter().filter(|&&keep| keep).count();
            proptest::prop_assert_eq!(out.kept.len(), want);
            proptest::prop_assert_eq!(out.map.0.len(), want);
            for (i, &source) in out.map.0.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let source = source as usize;
                proptest::prop_assert!(pattern[source]);
                proptest::prop_assert_eq!(&out.kept[i], &list[source]);
                if i > 0 {
                    proptest::prop_assert!(out.map.0[i - 1] < out.map.0[i]);
                }
            }
        }
    }

    // Golden hashes of both sealed outputs — holes included, since the hole
    // layout is part of the value.
    #[test]
    fn cull_determinism_golden_hash() {
        let source = slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]);
        let culled = cull(CullIn {
            list: source,
            pattern: vec![true, true, false, true, false],
        });
        assert_eq!(
            hex(culled.kept),
            "623d378981ca393e38108e901d30c497ec1bd826fd653a5ffb377cff65c59d2a"
        );
        assert_eq!(
            hex(culled.map),
            "bd577c78d544c5ce3c719a97f78ced7342a57a419777e0372592a4c26d44faad"
        );
    }
}
