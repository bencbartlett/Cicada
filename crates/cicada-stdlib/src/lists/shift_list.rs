//! The `shift_list` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`shift_list`].
#[derive(Ports, Clone, Debug)]
pub struct ShiftListIn {
    /// The list to shift.
    pub list: Vec<ElemSlot>,
    /// Shift amount: positive moves the head to the tail (`[a, b, c]` by
    /// `1` → `[b, c, a]`), negative the other way.
    pub offset: i64,
    /// Wrap the shifted-off slots around to the other end; off, they are
    /// dropped and the list shortens by `|offset|`.
    #[port(default = true)]
    pub wrap: bool,
}

/// Shift List — rotate a list by an offset (wrapping, the default) or slide
/// it and drop what falls off (`wrap=false`, GH Shift List). Slot-preserving
/// either way: absent slots move with the rest. A dropped tail or head
/// shifts no surviving index relative to its neighbours, so no index map is
/// returned — `cull` when provenance matters. Sliding past the end is not
/// an error: everything falls off and the list is empty, as GH's Shift List
/// does (stated here so the contract is true, not a hidden clamp).
///
/// # Returns
///
/// The shifted list — the same slot count when wrapping; otherwise `|offset|`
/// fewer, and empty once `|offset|` reaches the slot count.
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0]
/// rolled = shift_list(list=xs, offset=1)
/// clipped = shift_list(list=xs, offset=-1, wrap=False)
/// ```
#[node(category = "List & axis", tier = "1", version = 1, gh = "Shift List")]
#[must_use]
pub fn shift_list(input: ShiftListIn) -> Vec<ElemSlot> {
    let mut list = input.list;
    let len = list.len();
    if len == 0 {
        return list;
    }
    if input.wrap {
        #[allow(clippy::cast_possible_wrap)] // list lengths are far below i64::MAX
        let by = input.offset.rem_euclid(len as i64);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)] // 0 <= by < len
        list.rotate_left(by as usize);
        return list;
    }
    let dropped = input.offset.unsigned_abs().min(len as u64);
    #[allow(clippy::cast_possible_truncation)] // dropped <= len
    let dropped = dropped as usize;
    if input.offset >= 0 {
        list.drain(..dropped);
    } else {
        list.truncate(len - dropped);
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn shift_list_table_cases() {
        let xs = numbers(&[1.0, 2.0, 3.0, 4.0]);
        let shift = |offset, wrap| {
            shift_list(ShiftListIn {
                list: xs.clone(),
                offset,
                wrap,
            })
        };
        assert_eq!(shift(1, true), numbers(&[2.0, 3.0, 4.0, 1.0]));
        assert_eq!(shift(-1, true), numbers(&[4.0, 1.0, 2.0, 3.0]));
        assert_eq!(shift(5, true), numbers(&[2.0, 3.0, 4.0, 1.0]), "modular");
        assert_eq!(shift(0, true), xs, "zero is the identity");
        assert_eq!(shift(1, false), numbers(&[2.0, 3.0, 4.0]), "head dropped");
        assert_eq!(shift(-1, false), numbers(&[1.0, 2.0, 3.0]), "tail dropped");
        assert!(
            shift(9, false).is_empty(),
            "over-shifting drops everything — the `# Returns` contract's stated outcome"
        );
        assert!(
            shift(-4, false).is_empty(),
            "|offset| equal to the slot count empties the list either way"
        );
        assert_eq!(shift(-3, false), numbers(&[1.0]));
        // Holes move with the rest; the empty list shifts to itself.
        let holed = shift_list(ShiftListIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
            offset: 1,
            wrap: true,
        });
        assert_eq!(holed, slots(&[None, Some(3.0), Some(1.0)]));
        assert!(
            shift_list(ShiftListIn {
                list: vec![],
                offset: 3,
                wrap: false,
            })
            .is_empty()
        );
    }

    proptest::proptest! {
        // Wrapping: out[i] == in[(i + offset) mod n], and shifting back
        // restores the list. Not wrapping: the survivors are a contiguous
        // run of the source in order, n - min(|offset|, n) long.
        #[test]
        fn shift_list_property_rotation_and_clipping(
            values in holed_list(30),
            offset in -100i64..100,
        ) {
            let list = slots(&values);
            let n = list.len();
            let rolled = shift_list(ShiftListIn { list: list.clone(), offset, wrap: true });
            proptest::prop_assert_eq!(rolled.len(), n);
            for (i, slot) in rolled.iter().enumerate() {
                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let source = (i as i64 + offset).rem_euclid(n as i64) as usize;
                proptest::prop_assert_eq!(slot, &list[source]);
            }
            let back = shift_list(ShiftListIn { list: rolled, offset: -offset, wrap: true });
            proptest::prop_assert_eq!(&back, &list);
            let clipped = shift_list(ShiftListIn { list: list.clone(), offset, wrap: false });
            #[allow(clippy::cast_possible_truncation)]
            let dropped = (offset.unsigned_abs() as usize).min(n);
            proptest::prop_assert_eq!(clipped.len(), n - dropped);
            let start = if offset >= 0 { dropped } else { 0 };
            proptest::prop_assert_eq!(&clipped[..], &list[start..start + clipped.len()]);
        }
    }

    // Golden hash of the sealed output — holes included.
    #[test]
    fn shift_list_determinism_golden_hash() {
        let out = shift_list(ShiftListIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0)]),
            offset: 2,
            wrap: true,
        });
        assert_eq!(
            hex(out),
            "64e3bdf52604ba7d309e31ee9616b9ae2cb23c4148443e219d5701de1febbe57"
        );
    }
}
