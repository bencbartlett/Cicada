//! The `flatten` node.

use cicada_core::marshal::ElemSlot;
use cicada_macros::{Ports, node};

/// Inputs for [`flatten`].
#[derive(Ports, Clone, Debug)]
pub struct FlattenIn {
    /// The nested list.
    pub list: Vec<Vec<ElemSlot>>,
}

/// Flatten — one nesting level removed: the inner lists concatenated in
/// order (`[[a, b], [c]]` → `[a, b, c]`). One level only, always (docs/09:
/// `flatten_all` for every level, and says so); absent inner slots are
/// preserved as absent slots of the output.
///
/// # Panics
///
/// Panics when an OUTER slot is absent (a missing inner list — refused at
/// marshalling with its index: optional lists have no representation, so
/// there is nothing slot-preserving to do with one); the node itself has no
/// other refusal.
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0, 5.0]
/// pairs = chunk(list=xs, size=2)
/// flat = flatten(list=pairs)
/// ```
#[node(category = "List & axis", tier = "S", version = 1, gh = "Flatten Tree")]
#[must_use]
pub fn flatten(input: FlattenIn) -> Vec<ElemSlot> {
    input.list.into_iter().flatten().collect()
}

// Property coverage: the chunk ∘ flatten and partition ∘ flatten round-trip
// properties live with `chunk` and `partition`.
#[cfg(test)]
mod tests {
    use cicada_core::value::ValueData;

    use super::*;
    use crate::lists::chunk::{ChunkIn, chunk};
    use crate::lists::support::{data, hex, numbers, slots};

    #[test]
    fn flatten_table_cases() {
        let flat = flatten(FlattenIn {
            list: vec![numbers(&[1.0, 2.0]), vec![], slots(&[None, Some(3.0)])],
        });
        assert_eq!(flat.len(), 4);
        assert_eq!(data(&flat[0]), Some(&ValueData::Number(1.0)));
        assert_eq!(data(&flat[1]), Some(&ValueData::Number(2.0)));
        assert!(!flat[2].is_present(), "inner hole preserved in place");
        assert_eq!(data(&flat[3]), Some(&ValueData::Number(3.0)));
        // The empty outer list and all-empty inner lists both flatten to
        // the empty list.
        assert!(flatten(FlattenIn { list: vec![] }).is_empty());
        assert!(
            flatten(FlattenIn {
                list: vec![vec![], vec![]]
            })
            .is_empty()
        );
    }

    // The combinators reshape without re-sealing: the sealed output is a
    // golden hash — holes included, since the hole layout is part of the
    // value — and chunk ∘ flatten reproduces the source bytes.
    #[test]
    fn flatten_determinism_golden_hash() {
        let source = slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0)]);
        let groups = chunk(ChunkIn {
            list: source.clone(),
            size: 2,
        });
        let flat = flatten(FlattenIn { list: groups });
        assert_eq!(
            hex(flat.clone()),
            hex(source.clone()),
            "chunk ∘ flatten reproduces the source bytes"
        );
        assert_eq!(
            hex(flat),
            "c4b48c7eaebd8786527c5def53ec9bcc1884de412b17d7ff675bc2d3186f891a"
        );
    }
}
