//! The `duplicate` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

use crate::checked_count;

/// Inputs for [`duplicate`].
#[derive(Ports, Clone, Debug)]
pub struct DuplicateIn {
    /// The element to repeat (any kind — a geometry, a number, a text).
    pub item: ElemSlot,
    /// How many copies; `1` is the idiomatic singleton list.
    pub count: i64,
}

/// Duplicate — a list of `count` copies of one element (the idiomatic
/// singleton list is `count=1`: geometry lists come from nodes, list
/// literals hold scalars). An absent element duplicates into absent slots
/// (`E` carries the `?`).
///
/// # Returns
///
/// `count` copies of `item`, sharing one value.
///
/// # Panics
///
/// Panics when `count` is negative (`0` is the empty list) or above the
/// 2^22 slot ceiling (4,194,304 slots).
///
/// # Examples
///
/// ```cic
/// dot = construct_point(x=1.0, y=2.0, z=0.0)
/// dots = duplicate(item=dot, count=3)
/// ```
#[node(
    category = "List & axis",
    tier = "1",
    version = 2,
    gh = "Duplicate Data"
)]
#[must_use]
pub fn duplicate(input: DuplicateIn) -> Vec<ElemSlot> {
    let count = checked_count("duplicate", "count", input.count, 0, size_of::<ElemSlot>());
    vec![input.item; count]
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, hole, number};

    #[test]
    fn duplicate_table_cases() {
        let three = duplicate(DuplicateIn {
            item: number(4.5),
            count: 3,
        });
        assert_eq!(three.len(), 3);
        assert!(
            three
                .iter()
                .all(|slot| data(slot) == Some(&ValueData::Number(4.5)))
        );
        // The singleton and the empty list.
        assert_eq!(
            duplicate(DuplicateIn {
                item: number(1.0),
                count: 1,
            })
            .len(),
            1
        );
        assert!(
            duplicate(DuplicateIn {
                item: number(1.0),
                count: 0,
            })
            .is_empty()
        );
        // An absent element duplicates into absent slots.
        let holes = duplicate(DuplicateIn {
            item: hole(),
            count: 2,
        });
        assert_eq!(holes.len(), 2);
        assert!(holes.iter().all(|slot| !slot.is_present()));
    }

    #[test]
    #[should_panic(expected = "count must be >= 0")]
    fn duplicate_negative_count_is_red() {
        let _ = duplicate(DuplicateIn {
            item: number(1.0),
            count: -1,
        });
    }

    #[test]
    #[should_panic(
        expected = "duplicate: count is 4194305 — above the 4194304 (2^22) slot ceiling"
    )]
    fn duplicate_absurd_count_is_refused_not_allocated() {
        let _ = duplicate(DuplicateIn {
            item: number(1.0),
            count: crate::MAX_SLOTS + 1,
        });
    }

    proptest::proptest! {
        // Length is count; every slot is the same sealed value (same Arc).
        #[test]
        fn duplicate_property_count_copies_of_one_value(
            x in -1.0e6..1.0e6_f64,
            count in 0i64..50,
        ) {
            let item = number(x);
            let out = duplicate(DuplicateIn { item: item.clone(), count });
            let len = i64::try_from(out.len()).expect("count < 50 fits i64");
            proptest::prop_assert_eq!(len, count);
            for slot in &out {
                proptest::prop_assert_eq!(slot, &item);
            }
        }
    }

    // Golden hash of the sealed list — copies share one element hash, and
    // the list hash is the Merkle root over them.
    #[test]
    fn duplicate_determinism_golden_hash() {
        let out = duplicate(DuplicateIn {
            item: number(2.5),
            count: 4,
        });
        assert_eq!(
            hex(out),
            "780c78498dd88c2b832e4163414e6131f56d1eb1dcd1b2cdbb447ff422ef1828"
        );
    }
}
