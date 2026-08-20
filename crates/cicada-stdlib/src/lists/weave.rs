//! The `weave` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`weave`].
#[derive(Ports, Clone, Debug)]
pub struct WeaveIn {
    /// The turn order, repeated: `0` takes the next slot of `a`, `1` the
    /// next slot of `b` (`[0, 1]` alternates; `[0, 0, 1]` takes two from
    /// `a` per one from `b`).
    pub pattern: Vec<i64>,
    /// Stream 0.
    pub a: Vec<ElemSlot>,
    /// Stream 1.
    pub b: Vec<ElemSlot>,
}

/// Weave — merge two lists by a repeating turn pattern (`[0, 1]` over
/// `[a0, a1]` and `[b0, b1]` → `[a0, b0, a1, b1]`; GH Weave, two streams).
/// The rule, independent of the lengths: the output IS the repeated pattern
/// realized on the streams, cut where both are used up — so the two lengths
/// must be the turn counts of some prefix of the repeated pattern (`[0, 1]`
/// fits 3 + 2 or 2 + 2, never 4 + 2). A pair that does not fit is red at the
/// first turn that asks an exhausted stream while the other still has
/// slots (GH pads with nulls there; docs/09 retires silent matching).
/// Slot-preserving: absent slots take their turn like any other.
///
/// # Returns
///
/// The two streams interleaved in pattern order, every slot of both used
/// once.
///
/// # Panics
///
/// Panics when a pattern entry is not `0` or `1`, when the pattern is
/// empty but the streams are not, or when a turn asks a stream that is
/// already used up while the other still has slots left — the turn and
/// both remaining counts in the message (`pad_last` / `truncate` fit the
/// streams to the pattern).
///
/// # Examples
///
/// ```cic
/// evens = [0.0, 2.0, 4.0]
/// odds = [1.0, 3.0, 5.0]
/// counted = weave(pattern=[0, 1], a=evens, b=odds)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Weave")]
#[must_use]
pub fn weave(input: WeaveIn) -> Vec<ElemSlot> {
    let WeaveIn { pattern, a, b } = input;
    if let Some((entry, value)) = pattern
        .iter()
        .enumerate()
        .find(|&(_, &value)| value != 0 && value != 1)
    {
        panic!(
            "weave: pattern entry {entry} is {value} — a two-stream weave takes 0 (stream a) \
             or 1 (stream b)"
        );
    }
    let total = a.len() + b.len();
    if total == 0 {
        return Vec::new();
    }
    assert!(
        !pattern.is_empty(),
        "weave: the pattern is empty but the streams hold {total} slots — nothing says \
         whose turn it is"
    );
    let mut streams = [a.into_iter(), b.into_iter()];
    let mut out = Vec::with_capacity(total);
    for (turn, &stream) in pattern.iter().cycle().enumerate() {
        if out.len() == total {
            break;
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // 0 or 1, checked
        let index = stream as usize;
        let Some(slot) = streams[index].next() else {
            let names = ["a", "b"];
            panic!(
                "weave: turn {turn} (pattern entry {}) asks stream {} which has no slots \
                 left while stream {} still has {} — fit the streams to the pattern \
                 (`pad_last` / `truncate`)",
                turn % pattern.len(),
                names[index],
                names[1 - index],
                streams[1 - index].len()
            );
        };
        out.push(slot);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, numbers, slots};

    #[test]
    fn weave_table_cases() {
        // Alternating, equal lengths.
        let out = weave(WeaveIn {
            pattern: vec![0, 1],
            a: numbers(&[0.0, 2.0, 4.0]),
            b: numbers(&[1.0, 3.0, 5.0]),
        });
        assert_eq!(out, numbers(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]));
        // Two from a per one from b.
        let lopsided = weave(WeaveIn {
            pattern: vec![0, 0, 1],
            a: numbers(&[1.0, 2.0, 3.0, 4.0]),
            b: numbers(&[10.0, 20.0]),
        });
        assert_eq!(lopsided, numbers(&[1.0, 2.0, 10.0, 3.0, 4.0, 20.0]));
        // The pattern ends mid-cycle when both streams are used up.
        let partial = weave(WeaveIn {
            pattern: vec![0, 1],
            a: numbers(&[1.0, 2.0]),
            b: numbers(&[10.0]),
        });
        assert_eq!(partial, numbers(&[1.0, 10.0, 2.0]));
        // Holes take their turn.
        let holey = weave(WeaveIn {
            pattern: vec![1, 0],
            a: slots(&[None, Some(2.0)]),
            b: slots(&[Some(10.0), None]),
        });
        assert_eq!(holey, slots(&[Some(10.0), None, None, Some(2.0)]));
        // One stream empty: the pattern visits only the other.
        let solo = weave(WeaveIn {
            pattern: vec![0],
            a: numbers(&[1.0, 2.0]),
            b: vec![],
        });
        assert_eq!(solo, numbers(&[1.0, 2.0]));
        // Nothing to weave — even with an empty pattern.
        let nothing = weave(WeaveIn {
            pattern: vec![],
            a: vec![],
            b: vec![],
        });
        assert!(nothing.is_empty());
    }

    #[test]
    #[should_panic(expected = "pattern entry 1 is 2 — a two-stream weave takes 0")]
    fn weave_pattern_entry_outside_the_streams_is_red() {
        let _ = weave(WeaveIn {
            pattern: vec![0, 2],
            a: numbers(&[1.0]),
            b: numbers(&[2.0]),
        });
    }

    #[test]
    #[should_panic(expected = "pattern is empty but the streams hold 2 slots")]
    fn weave_empty_pattern_over_slots_is_red() {
        let _ = weave(WeaveIn {
            pattern: vec![],
            a: numbers(&[1.0]),
            b: numbers(&[2.0]),
        });
    }

    #[test]
    #[should_panic(
        expected = "turn 4 (pattern entry 0) asks stream a which has no slots left \
                               while stream b still has 1"
    )]
    fn weave_exhausted_stream_on_its_turn_is_red() {
        // a0 b0 a1 b1, then a's turn with nothing left and b2 waiting.
        let _ = weave(WeaveIn {
            pattern: vec![0, 1],
            a: numbers(&[1.0, 2.0]),
            b: numbers(&[10.0, 20.0, 30.0]),
        });
    }

    proptest::proptest! {
        // The weave IS the turn sequence realized on the streams: build the
        // sequence (k full cycles plus a prefix), size the streams to it,
        // and the output reads a's and b's slots in sequence order.
        #[test]
        fn weave_property_realizes_the_turn_sequence(
            pattern in proptest::collection::vec(0i64..2, 1..6),
            cycles in 0usize..4,
            prefix in 0usize..6,
        ) {
            let prefix = prefix % pattern.len();
            let sequence: Vec<i64> = pattern
                .iter()
                .cycle()
                .take(cycles * pattern.len() + prefix)
                .copied()
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let a: Vec<ElemSlot> = (0..sequence.iter().filter(|&&s| s == 0).count())
                .map(|i| crate::lists::support::number(i as f64))
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let b: Vec<ElemSlot> = (0..sequence.iter().filter(|&&s| s == 1).count())
                .map(|i| crate::lists::support::number(1000.0 + i as f64))
                .collect();
            let out = weave(WeaveIn { pattern, a: a.clone(), b: b.clone() });
            proptest::prop_assert_eq!(out.len(), sequence.len());
            let (mut ia, mut ib) = (a.iter(), b.iter());
            for (slot, &turn) in out.iter().zip(&sequence) {
                let want = if turn == 0 { ia.next() } else { ib.next() };
                proptest::prop_assert_eq!(Some(slot), want);
            }
        }
    }

    // Golden hash of the sealed output — holes included.
    #[test]
    fn weave_determinism_golden_hash() {
        let out = weave(WeaveIn {
            pattern: vec![0, 1, 1],
            a: slots(&[Some(1.0), None]),
            b: numbers(&[10.0, 20.0, 30.0, 40.0]),
        });
        assert_eq!(
            hex(out),
            "e6689279a1484ea02c7281760c618711ed34537da679d501971c59030ddad282"
        );
    }
}
