//! The `chunk` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

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
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0, 5.0]
/// pairs = chunk(list=xs, size=2)
/// ```
#[node(
    category = "List & axis",
    tier = "S",
    version = 1,
    gh = "Partition List"
)]
#[must_use]
pub fn chunk(input: ChunkIn) -> Vec<Vec<ElemSlot>> {
    let size = usize::try_from(input.size)
        .ok()
        .filter(|&size| size >= 1)
        .unwrap_or_else(|| panic!("chunk: size must be >= 1 (got {})", input.size));
    input.list.chunks(size).map(<[ElemSlot]>::to_vec).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::flatten::{FlattenIn, flatten};
    use crate::lists::support::{data, hex, holed_list, numbers, slots};

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

    proptest::proptest! {
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
    }

    // The combinators reshape without re-sealing: every present output
    // slot is the SAME Arc as its source (interning/instancing depend on
    // it), and the sealed output is a golden hash — holes included,
    // since the hole layout is part of the value.
    #[test]
    fn chunk_determinism_golden_hash() {
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
    }
}
