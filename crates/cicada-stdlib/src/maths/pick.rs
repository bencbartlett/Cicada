//! The `pick` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};
/// Inputs for [`pick`].
#[derive(Ports, Clone, Debug)]
pub struct PickIn {
    /// Which branch to take.
    pub pattern: bool,
    /// The value when the pattern is true (any kind; both branches bind one
    /// `E`).
    pub r#true: ElemSlot,
    /// The value when the pattern is false.
    pub r#false: ElemSlot,
}

/// Pick — the per-element `if`: `true` when the pattern holds, `false`
/// otherwise (lift with `each()` to choose per element; both branches are
/// solved — selection is data, not control flow).
///
/// # Returns
///
/// The chosen branch's value (absent when that branch is absent — `E`
/// carries the `?`).
///
/// # Examples
///
/// ```cic
/// tall = toggle(value=True)
/// height = pick(pattern=tall, true=3.0, false=1.0)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Pick'n'Choose"
)]
#[must_use]
pub fn pick(input: PickIn) -> ElemSlot {
    if input.pattern {
        input.r#true
    } else {
        input.r#false
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::support::{data, hex, hole, number};

    #[test]
    fn pick_table_cases() {
        let choose = |pattern| {
            pick(PickIn {
                pattern,
                r#true: number(3.0),
                r#false: number(1.0),
            })
        };
        assert_eq!(data(&choose(true)), Some(&ValueData::Number(3.0)));
        assert_eq!(data(&choose(false)), Some(&ValueData::Number(1.0)));
        // An absent branch selects as absent.
        assert!(
            !pick(PickIn {
                pattern: true,
                r#true: hole(),
                r#false: number(1.0),
            })
            .is_present()
        );
    }

    proptest::proptest! {
        // The output is exactly the selected branch's slot.
        #[test]
        fn pick_property_returns_the_selected_branch(
            pattern in proptest::bool::ANY,
            a in -1.0e6..1.0e6_f64,
            b in -1.0e6..1.0e6_f64,
        ) {
            let (yes, no) = (number(a), number(b));
            let got = pick(PickIn { pattern, r#true: yes.clone(), r#false: no.clone() });
            proptest::prop_assert_eq!(got, if pattern { yes } else { no });
        }
    }

    // pick passes the SAME sealed value through — hash-identical, the
    // determinism contract for a pass-through node.
    #[test]
    fn pick_determinism_passes_hash_through() {
        let chosen = number(4.25);
        let want = chosen.0.as_ref().unwrap().hash();
        let got = pick(PickIn {
            pattern: false,
            r#true: number(1.0),
            r#false: chosen,
        });
        assert_eq!(got.0.as_ref().unwrap().hash(), want);
        assert_eq!(
            hex(pick(PickIn {
                pattern: true,
                r#true: hole(),
                r#false: number(1.0),
            })),
            hex(ElemSlot(None)),
            "an absent selection seals to the one Nothing hash"
        );
    }
}
