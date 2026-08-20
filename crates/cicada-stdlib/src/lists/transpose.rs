//! The `transpose` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`transpose`].
#[derive(Ports, Clone, Debug)]
pub struct TransposeIn {
    /// The rectangular nested list (every inner list the same length).
    pub list: Vec<Vec<ElemSlot>>,
}

/// Transpose — swap the two nesting levels of a rectangular nested list
/// (`[[a, b], [c, d]]` → `[[a, c], [b, d]]`; GH Flip Matrix, docs/09).
/// Slot-preserving: absent slots move to their transposed places.
///
/// # Returns
///
/// The transposed nested list: as many groups as the inner lists were long,
/// each as long as there were inner lists.
///
/// # Panics
///
/// Panics when the inner lists differ in length — ragged data has no
/// transpose; the first offending group and both lengths are in the
/// message (never a silent pad or clip).
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
/// rows = chunk(list=xs, size=3)
/// columns = transpose(list=rows)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Flip Matrix")]
#[must_use]
pub fn transpose(input: TransposeIn) -> Vec<Vec<ElemSlot>> {
    let Some(width) = input.list.first().map(Vec::len) else {
        return Vec::new();
    };
    if let Some((row, group)) = input
        .list
        .iter()
        .enumerate()
        .find(|(_, group)| group.len() != width)
    {
        panic!(
            "transpose: group {row} has {} slots, group 0 has {width} — \
             the nested list must be rectangular",
            group.len()
        );
    }
    let mut columns: Vec<Vec<ElemSlot>> = (0..width)
        .map(|_| Vec::with_capacity(input.list.len()))
        .collect();
    for group in input.list {
        for (column, slot) in columns.iter_mut().zip(group) {
            column.push(slot);
        }
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, hole, number, numbers, slots};

    #[test]
    fn transpose_table_cases() {
        let out = transpose(TransposeIn {
            list: vec![
                slots(&[Some(1.0), Some(2.0), None]),
                numbers(&[4.0, 5.0, 6.0]),
            ],
        });
        assert_eq!(
            out,
            vec![
                numbers(&[1.0, 4.0]),
                numbers(&[2.0, 5.0]),
                vec![hole(), number(6.0)],
            ]
        );
        // Empty outer list, and inner lists of zero length: nothing to swap.
        assert!(transpose(TransposeIn { list: vec![] }).is_empty());
        assert!(
            transpose(TransposeIn {
                list: vec![vec![], vec![]],
            })
            .is_empty()
        );
        // One row becomes one column.
        assert_eq!(
            transpose(TransposeIn {
                list: vec![numbers(&[1.0, 2.0])],
            }),
            vec![numbers(&[1.0]), numbers(&[2.0])]
        );
    }

    #[test]
    #[should_panic(expected = "group 1 has 1 slots, group 0 has 2")]
    fn transpose_ragged_is_red() {
        let _ = transpose(TransposeIn {
            list: vec![numbers(&[1.0, 2.0]), numbers(&[3.0])],
        });
    }

    // A LONGER later row is ragged too: a `<` check would silently drop its
    // tail (the 5 below) — the exact silent fallback the rules forbid (C1
    // review: the shorter-row case alone let that mutation pass).
    #[test]
    #[should_panic(expected = "group 1 has 3 slots, group 0 has 2")]
    fn transpose_longer_later_row_is_red() {
        let _ = transpose(TransposeIn {
            list: vec![numbers(&[1.0, 2.0]), numbers(&[3.0, 4.0, 5.0])],
        });
    }

    proptest::proptest! {
        // Shape swaps (rows × cols → cols × rows), out[j][i] == in[i][j], and
        // transpose is an involution on non-empty rectangles.
        #[test]
        fn transpose_property_is_an_involution(
            rows in 1usize..6,
            cols in 1usize..6,
            seed in proptest::collection::vec(proptest::option::of(-1.0e6..1.0e6_f64), 36),
        ) {
            let grid: Vec<Vec<ElemSlot>> = (0..rows)
                .map(|r| slots(&seed[r * cols..(r + 1) * cols]))
                .collect();
            let out = transpose(TransposeIn { list: grid.clone() });
            proptest::prop_assert_eq!(out.len(), cols);
            for (j, column) in out.iter().enumerate() {
                proptest::prop_assert_eq!(column.len(), rows);
                for (i, slot) in column.iter().enumerate() {
                    proptest::prop_assert_eq!(slot, &grid[i][j]);
                }
            }
            proptest::prop_assert_eq!(transpose(TransposeIn { list: out }), grid);
        }
    }

    // Golden hash of the sealed nested output — holes included.
    #[test]
    fn transpose_determinism_golden_hash() {
        let out = transpose(TransposeIn {
            list: vec![
                slots(&[Some(1.0), None, Some(3.0)]),
                numbers(&[4.0, 5.0, 6.0]),
            ],
        });
        assert_eq!(
            hex(out),
            "7717d0e62a967d6deb2b28645190cdd3b137f5a44b5e008c5288d2f4e7a808cc"
        );
    }
}
