//! The `truncate` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`truncate`].
#[derive(Ports, Clone, Debug)]
pub struct TruncateIn {
    /// The list to shorten.
    pub list: Vec<ElemSlot>,
    /// The slot count to keep (typically `length` of the list it will be
    /// zipped with).
    pub count: i64,
}

/// Truncate — shorten a list to its first `count` slots: the explicit,
/// visible form of Grasshopper's shortest-list matching (docs/09 — zip is
/// strict; this is the opt-in adapter). Dropping a tail shifts no surviving
/// index, so no index map is returned — `cull` when elements leave from
/// the middle.
///
/// # Returns
///
/// The first `count` slots (unchanged when `count` equals the length).
///
/// # Panics
///
/// Panics when `count` exceeds the list's slot count (`truncate` only
/// shortens — `pad_last` lengthens) or is negative.
///
/// # Examples
///
/// ```cic
/// few = [1.0, 2.0]
/// many = [10.0, 20.0, 30.0, 40.0]
/// n = length(list=few)
/// clipped = truncate(list=many, count=n)
/// sums = add(a=each(few), b=each(clipped))
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = none)]
#[must_use]
pub fn truncate(input: TruncateIn) -> Vec<ElemSlot> {
    let mut list = input.list;
    let count = usize::try_from(input.count)
        .unwrap_or_else(|_| panic!("truncate: count must be >= 0, got {}", input.count));
    assert!(
        count <= list.len(),
        "truncate: count {count} exceeds the list's {} slots — truncate only shortens \
         (pad_last lengthens)",
        list.len()
    );
    list.truncate(count);
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn truncate_table_cases() {
        assert_eq!(
            truncate(TruncateIn {
                list: numbers(&[1.0, 2.0, 3.0, 4.0]),
                count: 2,
            }),
            numbers(&[1.0, 2.0])
        );
        assert_eq!(
            truncate(TruncateIn {
                list: numbers(&[1.0, 2.0]),
                count: 2,
            }),
            numbers(&[1.0, 2.0]),
            "count == length is the identity"
        );
        assert!(
            truncate(TruncateIn {
                list: numbers(&[1.0, 2.0]),
                count: 0,
            })
            .is_empty(),
            "zero keeps nothing"
        );
        assert_eq!(
            truncate(TruncateIn {
                list: slots(&[Some(1.0), None, Some(3.0)]),
                count: 2,
            }),
            slots(&[Some(1.0), None]),
            "a kept hole stays a hole"
        );
    }

    #[test]
    #[should_panic(expected = "count 5 exceeds the list's 3 slots")]
    fn truncate_lengthening_is_red() {
        let _ = truncate(TruncateIn {
            list: numbers(&[1.0, 2.0, 3.0]),
            count: 5,
        });
    }

    #[test]
    #[should_panic(expected = "count must be >= 0")]
    fn truncate_negative_count_is_red() {
        let _ = truncate(TruncateIn {
            list: numbers(&[1.0]),
            count: -1,
        });
    }

    proptest::proptest! {
        // The output is exactly the source's first `count` slots.
        #[test]
        fn truncate_property_keeps_the_prefix(
            values in holed_list(30),
            fraction in 0.0..=1.0_f64,
        ) {
            let list = slots(&values);
            #[allow(clippy::cast_precision_loss, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let count = ((list.len() as f64) * fraction).floor() as usize;
            let out = truncate(TruncateIn {
                list: list.clone(),
                count: i64::try_from(count).expect("small"),
            });
            proptest::prop_assert_eq!(&out[..], &list[..count]);
        }
    }

    // Golden hash of the sealed output.
    #[test]
    fn truncate_determinism_golden_hash() {
        let out = truncate(TruncateIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]),
            count: 3,
        });
        assert_eq!(
            hex(out),
            "3c03166ac4b61dccf078fa869b66dda7e5fec22449d27ffb7f897a623014baf8"
        );
    }
}
