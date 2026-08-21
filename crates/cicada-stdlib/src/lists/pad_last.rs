//! The `pad_last` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

use crate::checked_count;

/// Inputs for [`pad_last`].
#[derive(Ports, Clone, Debug)]
pub struct PadLastIn {
    /// The list to lengthen.
    pub list: Vec<ElemSlot>,
    /// The slot count to reach (typically `length` of the list it will be
    /// zipped with).
    pub count: i64,
}

/// Pad Last — lengthen a list to `count` slots by repeating its last slot:
/// the explicit, visible form of Grasshopper's longest-list matching
/// (docs/09 — zip is strict; this is the opt-in adapter, and the repeated
/// element shows up in the text). An absent last slot pads with absent
/// slots.
///
/// # Returns
///
/// The list with its last slot repeated up to `count` slots (unchanged when
/// `count` equals its length).
///
/// # Panics
///
/// Panics when `count` is below the list's slot count (`pad_last` only
/// lengthens — `truncate` shortens) or above the 2^22 slot ceiling
/// (4,194,304 slots), or when the list is empty and `count` is positive (no
/// last slot to repeat).
///
/// # Examples
///
/// ```cic
/// few = [1.0, 2.0]
/// many = [10.0, 20.0, 30.0, 40.0]
/// n = length(list=many)
/// padded = pad_last(list=few, count=n)
/// sums = add(a=each(padded), b=each(many))
/// ```
#[node(category = "List & axis", tier = "S", version = 2, gh = "Longest List")]
#[must_use]
pub fn pad_last(input: PadLastIn) -> Vec<ElemSlot> {
    let mut list = input.list;
    let count = checked_count("pad_last", "count", input.count, 0, size_of::<ElemSlot>());
    assert!(
        count >= list.len(),
        "pad_last: count {count} is below the list's {} slots — pad_last only lengthens \
         (truncate shortens)",
        list.len()
    );
    let Some(filler) = list.last().cloned() else {
        assert!(
            count == 0,
            "pad_last: the list is empty — no last slot to repeat up to {count}"
        );
        return list;
    };
    list.resize(count, filler);
    list
}

#[cfg(test)]
mod tests {
    use proptest::strategy::Strategy as _;

    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn pad_last_table_cases() {
        assert_eq!(
            pad_last(PadLastIn {
                list: numbers(&[1.0, 2.0]),
                count: 4,
            }),
            numbers(&[1.0, 2.0, 2.0, 2.0])
        );
        assert_eq!(
            pad_last(PadLastIn {
                list: numbers(&[1.0, 2.0]),
                count: 2,
            }),
            numbers(&[1.0, 2.0]),
            "count == length is the identity"
        );
        assert_eq!(
            pad_last(PadLastIn {
                list: slots(&[Some(1.0), None]),
                count: 3,
            }),
            slots(&[Some(1.0), None, None]),
            "an absent last slot pads with absent slots"
        );
        assert!(
            pad_last(PadLastIn {
                list: vec![],
                count: 0,
            })
            .is_empty()
        );
    }

    #[test]
    #[should_panic(expected = "count 1 is below the list's 3 slots")]
    fn pad_last_shortening_is_red() {
        let _ = pad_last(PadLastIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            count: 1,
        });
    }

    #[test]
    #[should_panic(expected = "the list is empty")]
    fn pad_last_empty_list_is_red() {
        let _ = pad_last(PadLastIn {
            list: vec![],
            count: 2,
        });
    }

    // One past the ceiling pins where the guard sits and what it says (a
    // guard moved after the `resize` would still pass this — 64 MiB of
    // slots is buildable); the absurd case below is what detects that
    // mutation.
    #[test]
    #[should_panic(
        expected = "pad_last: count is 4194305 — above the 4194304 (2^22) slot ceiling of one \
                    node output"
    )]
    fn pad_last_one_past_the_ceiling_is_red() {
        let _ = pad_last(PadLastIn {
            list: numbers(&[1.0]),
            count: crate::MAX_SLOTS + 1,
        });
    }

    // The absurd count a literal or an Integer wire can carry: a `resize`
    // to 10^11 slots is a 1.6 TB buffer no machine holds — with the guard
    // after it this test binary would abort on allocation failure
    // (`catch_unwind` cannot catch that), so passing proves the refusal
    // precedes the allocation.
    #[test]
    #[should_panic(
        expected = "pad_last: count is 100000000000 — above the 4194304 (2^22) slot ceiling of \
                    one node output"
    )]
    fn pad_last_absurd_count_is_refused_not_allocated() {
        let _ = pad_last(PadLastIn {
            list: numbers(&[1.0]),
            count: 100_000_000_000,
        });
    }

    proptest::proptest! {
        // Length is count; the prefix is the source; the padding is the
        // source's last slot.
        #[test]
        fn pad_last_property_repeats_the_last_slot(
            values in holed_list(30).prop_filter("non-empty", |v| !v.is_empty()),
            extra in 0usize..20,
        ) {
            let list = slots(&values);
            let count = list.len() + extra;
            let out = pad_last(PadLastIn {
                list: list.clone(),
                count: i64::try_from(count).expect("small"),
            });
            proptest::prop_assert_eq!(out.len(), count);
            proptest::prop_assert_eq!(&out[..list.len()], &list[..]);
            for slot in &out[list.len()..] {
                proptest::prop_assert_eq!(slot, list.last().expect("non-empty"));
            }
        }
    }

    // Golden hash of the sealed output.
    #[test]
    fn pad_last_determinism_golden_hash() {
        let out = pad_last(PadLastIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
            count: 5,
        });
        assert_eq!(
            hex(out),
            "e454d1e7f907f3aea983a19ae645bfc1ec7dc37c364a8c4f05ab4b0602cac000"
        );
    }
}
