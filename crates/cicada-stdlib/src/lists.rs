//! List & axis nodes (docs/08 §Catalog 4, docs/09 combinator inventory).
//! The wire-level combinators (`each()` map/zip) live in the dialect; these
//! are the node forms the spike needs. Element kinds flow through the `E`
//! type variable — `item` of a `[Point]` is a `Point`, `flatten` of a
//! `[[Point]]` is a `[Point]`, statically — and `E` carries element
//! optionality with it: every node here is slot-preserving (docs/08 rule
//! 6), so a `[Point?]` flows through as `[Point?]` and absent slots keep
//! their places. Elements leave a list only through `cull` (and the
//! Optional-flow `compact`, v0.1), which return an `IndexMap` so identity
//! survives.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
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
#[node(category = "List & axis", tier = "S", version = 2)]
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

/// Inputs for [`length`].
#[derive(Ports, Clone, Debug)]
pub struct LengthIn {
    /// The list — absent (`Optional`) slots count too.
    pub list: Vec<ElemSlot>,
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

/// Inputs for [`flatten`].
#[derive(Ports, Clone, Debug)]
pub struct FlattenIn {
    /// The nested list.
    pub list: Vec<Vec<ElemSlot>>,
}

/// Flatten — one nesting level removed: the inner lists concatenated in
/// order (`[[a, b], [c]]` → `[a, b, c]`). One level only, always (docs/09:
/// `flatten_all` for every level, and says so); absent inner slots are
/// preserved as absent slots of the output.
///
/// # Panics
///
/// Panics when an OUTER slot is absent (a missing inner list — refused at
/// marshalling with its index: optional lists have no representation, so
/// there is nothing slot-preserving to do with one); the node itself has no
/// other refusal.
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn flatten(input: FlattenIn) -> Vec<ElemSlot> {
    input.list.into_iter().flatten().collect()
}

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

/// Inputs for [`chunk`].
#[derive(Ports, Clone, Debug)]
pub struct ChunkIn {
    /// The list to split.
    pub list: Vec<ElemSlot>,
    /// Group size (the last group may be shorter).
    pub size: i64,
}

/// Chunk — consecutive groups of `size` slots; the last group may be short
/// (GH Partition List). Slot-preserving: absent slots land in their group
/// as absent slots; an empty list chunks to no groups.
///
/// # Panics
///
/// Panics when `size < 1`.
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn chunk(input: ChunkIn) -> Vec<Vec<ElemSlot>> {
    let size = usize::try_from(input.size)
        .ok()
        .filter(|&size| size >= 1)
        .unwrap_or_else(|| panic!("chunk: size must be >= 1 (got {})", input.size));
    input.list.chunks(size).map(<[ElemSlot]>::to_vec).collect()
}

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
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn concat(input: ConcatIn) -> Vec<ElemSlot> {
    let mut out = input.a;
    out.extend(input.b);
    out
}

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
#[node(category = "List & axis", tier = "S", version = 1)]
#[must_use]
pub fn cull(input: CullIn) -> CullOut {
    assert!(
        input.pattern.len() == input.list.len(),
        "cull: pattern has {} entries for a list of {} slots — zip is strict \
         (pad_last / cycle / truncate are the opt-in policies)",
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
    use std::sync::Arc;

    use cicada_core::marshal::IntoValue;
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn number(x: f64) -> ElemSlot {
        ElemSlot(Some(HashedValue::new(ValueData::Number(x)).unwrap()))
    }

    fn hole() -> ElemSlot {
        ElemSlot(None)
    }

    fn numbers(values: &[f64]) -> Vec<ElemSlot> {
        values.iter().map(|&x| number(x)).collect()
    }

    /// A list from `Some(x)` = number, `None` = absent slot.
    fn slots(values: &[Option<f64>]) -> Vec<ElemSlot> {
        values.iter().map(|v| v.map_or_else(hole, number)).collect()
    }

    fn data(slot: &ElemSlot) -> Option<&ValueData> {
        slot.0.as_deref().map(HashedValue::data)
    }

    fn hex<V: IntoValue>(value: V) -> String {
        value.into_value().unwrap().hash().to_hex()
    }

    // Proptest strategy: a list with holes, drawn as Option<f64> per slot.
    fn holed_list(max: usize) -> impl proptest::strategy::Strategy<Value = Vec<Option<f64>>> {
        proptest::collection::vec(proptest::option::of(-1.0e6..1.0e6_f64), 0..max)
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

    #[test]
    fn length_counts_slots_including_holes() {
        assert_eq!(length(LengthIn { list: vec![] }), 0);
        assert_eq!(
            length(LengthIn {
                list: slots(&[Some(1.0), None, Some(2.0)]),
            }),
            3,
            "absent slots keep their places"
        );
    }

    #[test]
    fn flatten_table_cases() {
        let flat = flatten(FlattenIn {
            list: vec![numbers(&[1.0, 2.0]), vec![], slots(&[None, Some(3.0)])],
        });
        assert_eq!(flat.len(), 4);
        assert_eq!(data(&flat[0]), Some(&ValueData::Number(1.0)));
        assert_eq!(data(&flat[1]), Some(&ValueData::Number(2.0)));
        assert!(!flat[2].is_present(), "inner hole preserved in place");
        assert_eq!(data(&flat[3]), Some(&ValueData::Number(3.0)));
        // The empty outer list and all-empty inner lists both flatten to
        // the empty list.
        assert!(flatten(FlattenIn { list: vec![] }).is_empty());
        assert!(
            flatten(FlattenIn {
                list: vec![vec![], vec![]]
            })
            .is_empty()
        );
    }

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

    #[test]
    fn chunk_table_cases() {
        let groups = chunk(ChunkIn {
            list: slots(&[Some(1.0), Some(2.0), None, Some(4.0), Some(5.0)]),
            size: 2,
        });
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[1].len(), 2);
        assert_eq!(groups[2].len(), 1, "the last group is short");
        assert!(!groups[1][0].is_present(), "the hole lands in its group");
        assert_eq!(data(&groups[2][0]), Some(&ValueData::Number(5.0)));
        // Exact multiple: no short group; empty list: no groups.
        assert_eq!(
            chunk(ChunkIn {
                list: numbers(&[1.0, 2.0, 3.0, 4.0]),
                size: 2,
            })
            .len(),
            2
        );
        assert!(
            chunk(ChunkIn {
                list: vec![],
                size: 3
            })
            .is_empty()
        );
        // A size beyond the length is one group.
        assert_eq!(
            chunk(ChunkIn {
                list: numbers(&[1.0, 2.0]),
                size: 10,
            })
            .len(),
            1
        );
    }

    #[test]
    #[should_panic(expected = "size must be >= 1 (got 0)")]
    fn chunk_zero_size_is_red() {
        let _ = chunk(ChunkIn {
            list: numbers(&[1.0]),
            size: 0,
        });
    }

    #[test]
    #[should_panic(expected = "size must be >= 1 (got -3)")]
    fn chunk_negative_size_is_red() {
        let _ = chunk(ChunkIn {
            list: vec![],
            size: -3,
        });
    }

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

        // length counts every slot — absent slots included.
        #[test]
        fn length_property_counts_all_slots(values in holed_list(40)) {
            let want = i64::try_from(values.len()).expect("len < 40 fits i64");
            proptest::prop_assert_eq!(length(LengthIn { list: slots(&values) }), want);
        }

        // chunk then flatten is the identity (holes included), and every
        // group but the last is exactly `size` long.
        #[test]
        fn chunk_flatten_property_roundtrip(values in holed_list(40), size in 1i64..12) {
            let list = slots(&values);
            let groups = chunk(ChunkIn { list: list.clone(), size });
            let size = usize::try_from(size).expect("1..12 fits usize");
            proptest::prop_assert_eq!(groups.len(), values.len().div_ceil(size));
            for group in groups.iter().take(groups.len().saturating_sub(1)) {
                proptest::prop_assert_eq!(group.len(), size);
            }
            if let Some(last) = groups.last() {
                proptest::prop_assert!(!last.is_empty() && last.len() <= size);
            }
            proptest::prop_assert_eq!(flatten(FlattenIn { list: groups }), list);
        }

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

    #[test]
    fn length_determinism_golden_hash() {
        let list: Vec<ElemSlot> = (0..7).map(|i| number(f64::from(i))).collect();
        let out = length(LengthIn { list });
        assert_eq!(
            hex(out),
            "64ad282ae443e2988c185814d9431a6fda5e3053a917ebfffb9076953f9b2d3a"
        );
    }

    // The combinators reshape without re-sealing: every present output
    // slot is the SAME Arc as its source (interning/instancing depend on
    // it), and the sealed outputs are golden hashes — holes included,
    // since the hole layout is part of the value.
    #[test]
    fn combinators_determinism_golden_hashes() {
        let source = slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]);
        let groups = chunk(ChunkIn {
            list: source.clone(),
            size: 2,
        });
        assert!(Arc::ptr_eq(
            groups[0][0].0.as_ref().unwrap(),
            source[0].0.as_ref().unwrap()
        ));
        assert_eq!(
            hex(groups.clone()),
            "93b4d63e84429f7e95117e588ed92d130514bef7913e4ca9b415162fa061ad5d"
        );
        let flat = flatten(FlattenIn { list: groups });
        assert_eq!(
            hex(flat.clone()),
            hex(source.clone()),
            "chunk ∘ flatten reproduces the source bytes"
        );
        assert_eq!(
            hex(flat),
            "c4b48c7eaebd8786527c5def53ec9bcc1884de412b17d7ff675bc2d3186f891a"
        );
        let parts = partition(PartitionIn {
            list: source.clone(),
            sizes: vec![2, 0, 3],
        });
        assert_eq!(
            hex(parts),
            "68b633aabc1c7c343da607d71869085d4a9091cc5fa787d1f629fd38ed007e3c"
        );
        let joined = concat(ConcatIn {
            a: source.clone(),
            b: numbers(&[6.0]),
        });
        assert_eq!(
            hex(joined),
            "a5cab35bd52b2e140e6fb6bd4a6f96c128a532c69475fb306fff47d6d7397437"
        );
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
