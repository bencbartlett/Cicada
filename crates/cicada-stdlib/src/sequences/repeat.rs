//! The `repeat` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

use crate::slot_count;

/// Inputs for [`repeat`].
#[derive(Ports, Clone, Debug)]
pub struct RepeatIn {
    /// The pattern to cycle (any kind; absent slots cycle too).
    pub pattern: Vec<ElemSlot>,
    /// The slot count to produce (typically `length` of the list it will be
    /// zipped with).
    pub count: i64,
}

/// Repeat — cycle a pattern until it is `count` slots long (`[a, b]` to
/// `5` → `[a, b, a, b, a]`; GH Repeat Data): the explicit, visible form of
/// the cyclic zip policy (docs/09 — zip is strict; this is the opt-in
/// adapter). Slot-preserving: absent slots repeat in their turn.
///
/// # Returns
///
/// `count` slots, the pattern repeated in order and cut at `count`.
///
/// # Panics
///
/// Panics when `count` is negative or above the 2^24 slot ceiling
/// (16,777,216 slots), or when the pattern is empty and `count` is positive
/// (nothing to repeat).
///
/// # Examples
///
/// ```cic
/// on_off = [True, False]
/// mask = repeat(pattern=on_off, count=5)
/// ```
#[node(
    category = "Sequences & random",
    tier = "1",
    version = 1,
    gh = "Repeat Data"
)]
#[must_use]
pub fn repeat(input: RepeatIn) -> Vec<ElemSlot> {
    let count = slot_count("repeat", "count", input.count, 0);
    if count == 0 {
        return Vec::new();
    }
    assert!(
        !input.pattern.is_empty(),
        "repeat: the pattern is empty — nothing to repeat up to {count} slots"
    );
    input.pattern.iter().cycle().take(count).cloned().collect()
}

#[cfg(test)]
mod tests {
    use proptest::strategy::Strategy as _;

    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn repeat_table_cases() {
        assert_eq!(
            repeat(RepeatIn {
                pattern: numbers(&[1.0, 2.0]),
                count: 5,
            }),
            numbers(&[1.0, 2.0, 1.0, 2.0, 1.0])
        );
        assert_eq!(
            repeat(RepeatIn {
                pattern: numbers(&[1.0, 2.0, 3.0]),
                count: 2,
            }),
            numbers(&[1.0, 2.0]),
            "a count below the pattern length cuts it"
        );
        assert_eq!(
            repeat(RepeatIn {
                pattern: slots(&[Some(1.0), None]),
                count: 3,
            }),
            slots(&[Some(1.0), None, Some(1.0)]),
            "holes repeat in their turn"
        );
        assert!(
            repeat(RepeatIn {
                pattern: vec![],
                count: 0,
            })
            .is_empty(),
            "zero of nothing is the empty list"
        );
    }

    #[test]
    #[should_panic(expected = "the pattern is empty")]
    fn repeat_empty_pattern_is_red() {
        let _ = repeat(RepeatIn {
            pattern: vec![],
            count: 3,
        });
    }

    #[test]
    #[should_panic(expected = "count must be >= 0")]
    fn repeat_negative_count_is_red() {
        let _ = repeat(RepeatIn {
            pattern: numbers(&[1.0]),
            count: -1,
        });
    }

    #[test]
    #[should_panic(expected = "repeat: count is 16777217 — above the 16777216 (2^24) slot ceiling")]
    fn repeat_absurd_count_is_refused_not_allocated() {
        let _ = repeat(RepeatIn {
            pattern: numbers(&[1.0]),
            count: crate::MAX_SLOTS + 1,
        });
    }

    proptest::proptest! {
        // Length is count; out[i] == pattern[i mod n].
        #[test]
        fn repeat_property_cycles_the_pattern(
            values in holed_list(10).prop_filter("non-empty", |v| !v.is_empty()),
            count in 0i64..60,
        ) {
            let pattern = slots(&values);
            let out = repeat(RepeatIn { pattern: pattern.clone(), count });
            let len = i64::try_from(out.len()).expect("count < 60 fits i64");
            proptest::prop_assert_eq!(len, count);
            for (i, slot) in out.iter().enumerate() {
                proptest::prop_assert_eq!(slot, &pattern[i % pattern.len()]);
            }
        }
    }

    // Golden hash of the sealed output — holes included.
    #[test]
    fn repeat_determinism_golden_hash() {
        let out = repeat(RepeatIn {
            pattern: slots(&[Some(1.0), None, Some(3.0)]),
            count: 7,
        });
        assert_eq!(
            hex(out),
            "e8181f35d4c42ef556dc07eed955643cf2ec3386e35b67358b293953d340429c"
        );
    }
}
