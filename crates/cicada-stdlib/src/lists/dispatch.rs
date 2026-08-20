//! The `dispatch` node.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
use cicada_macros::{Ports, node};

/// Inputs for [`dispatch`].
#[derive(Ports, Clone, Debug)]
pub struct DispatchIn {
    /// The list to split.
    pub list: Vec<ElemSlot>,
    /// Where true the slot goes to `a`, where false to `b` — one entry per
    /// slot (strict zip; no pattern repetition).
    pub pattern: Vec<bool>,
}

/// Outputs of [`dispatch`].
#[derive(Ports, Clone, Debug)]
pub struct DispatchOut {
    /// The slots whose pattern entry is true, in order.
    pub a: Vec<ElemSlot>,
    /// The slots whose pattern entry is false, in order.
    pub b: Vec<ElemSlot>,
    /// Provenance of `a`: `map_a[i]` is the source index of `a[i]`.
    pub map_a: IndexMap,
    /// Provenance of `b`: `map_b[i]` is the source index of `b[i]`.
    pub map_b: IndexMap,
}

/// Dispatch — split a list into two by a boolean pattern (true → `a`,
/// false → `b`), each with its index map back into the source; `a` and `b`
/// together hold every slot (absent slots dispatch like any other).
///
/// # Panics
///
/// Panics when the pattern's length differs from the list's slot count —
/// strict zip, both counts in the message.
///
/// # Examples
///
/// ```cic
/// xs = [10.0, 20.0, 30.0, 40.0]
/// odd = [True, False, True, False]
/// picked, rest, picked_from, rest_from = dispatch(list=xs, pattern=odd)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Dispatch")]
#[must_use]
pub fn dispatch(input: DispatchIn) -> DispatchOut {
    assert!(
        input.pattern.len() == input.list.len(),
        "dispatch: pattern has {} entries for a list of {} slots — zip is strict",
        input.pattern.len(),
        input.list.len()
    );
    let mut out = DispatchOut {
        a: Vec::new(),
        b: Vec::new(),
        map_a: IndexMap(Vec::new()),
        map_b: IndexMap(Vec::new()),
    };
    for (index, (slot, to_a)) in input.list.into_iter().zip(input.pattern).enumerate() {
        if to_a {
            out.a.push(slot);
            out.map_a.0.push(index as u64);
        } else {
            out.b.push(slot);
            out.map_b.0.push(index as u64);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

    #[test]
    fn dispatch_table_cases() {
        let out = dispatch(DispatchIn {
            list: slots(&[Some(10.0), None, Some(30.0), Some(40.0)]),
            pattern: vec![true, true, false, true],
        });
        assert_eq!(out.a.len(), 3);
        assert_eq!(data(&out.a[0]), Some(&ValueData::Number(10.0)));
        assert!(!out.a[1].is_present(), "a hole dispatches like any slot");
        assert_eq!(data(&out.a[2]), Some(&ValueData::Number(40.0)));
        assert_eq!(out.map_a, IndexMap(vec![0, 1, 3]));
        assert_eq!(out.b.len(), 1);
        assert_eq!(data(&out.b[0]), Some(&ValueData::Number(30.0)));
        assert_eq!(out.map_b, IndexMap(vec![2]));
        // All one way: the other side is empty. Empty in, empty out.
        let all_b = dispatch(DispatchIn {
            list: numbers(&[1.0, 2.0]),
            pattern: vec![false, false],
        });
        assert!(all_b.a.is_empty() && all_b.map_a.0.is_empty());
        assert_eq!(all_b.map_b, IndexMap(vec![0, 1]));
        let empty = dispatch(DispatchIn {
            list: vec![],
            pattern: vec![],
        });
        assert!(empty.a.is_empty() && empty.b.is_empty());
    }

    #[test]
    #[should_panic(expected = "pattern has 2 entries for a list of 3 slots")]
    fn dispatch_pattern_length_mismatch_is_red() {
        let _ = dispatch(DispatchIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            pattern: vec![true, false],
        });
    }

    proptest::proptest! {
        // a and b partition the list: their maps are disjoint, increasing,
        // and cover 0..n together; each output slot is its source slot.
        #[test]
        fn dispatch_property_partitions_the_list(
            values in holed_list(30),
            seed in proptest::collection::vec(proptest::bool::ANY, 30),
        ) {
            let list = slots(&values);
            let pattern: Vec<bool> = seed[..values.len()].to_vec();
            let out = dispatch(DispatchIn { list: list.clone(), pattern: pattern.clone() });
            proptest::prop_assert_eq!(out.a.len() + out.b.len(), list.len());
            let mut covered = vec![false; list.len()];
            for (side, map, want) in [(&out.a, &out.map_a, true), (&out.b, &out.map_b, false)] {
                proptest::prop_assert_eq!(side.len(), map.0.len());
                for (i, &source) in map.0.iter().enumerate() {
                    #[allow(clippy::cast_possible_truncation)]
                    let source = source as usize;
                    proptest::prop_assert_eq!(pattern[source], want);
                    proptest::prop_assert!(!covered[source]);
                    covered[source] = true;
                    proptest::prop_assert_eq!(&side[i], &list[source]);
                    if i > 0 {
                        proptest::prop_assert!(map.0[i - 1] < map.0[i]);
                    }
                }
            }
            proptest::prop_assert!(covered.iter().all(|&c| c));
        }
    }

    // Golden hashes of the four sealed outputs — holes included.
    #[test]
    fn dispatch_determinism_golden_hash() {
        let out = dispatch(DispatchIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]),
            pattern: vec![true, true, false, true, false],
        });
        assert_eq!(
            hex(out.a),
            "623d378981ca393e38108e901d30c497ec1bd826fd653a5ffb377cff65c59d2a"
        );
        assert_eq!(
            hex(out.b),
            "357a8353595e5f9a47477928e55ffa2d8bb6bb1ae62090baf03f02506206de63"
        );
        assert_eq!(
            hex(out.map_a),
            "bd577c78d544c5ce3c719a97f78ced7342a57a419777e0372592a4c26d44faad"
        );
        assert_eq!(
            hex(out.map_b),
            "fa62247a7445075b2719782aadc0ed28593032b4ea61cce7adecebeb6bf745c6"
        );
    }
}
