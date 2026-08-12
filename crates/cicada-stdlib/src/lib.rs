//! The Cicada standard library: the node catalog (docs/08).
//!
//! Nodes are pure functions — the scheduler calls *them*; this crate never
//! depends on `cicada-sched` (doc 14, enforced by the dependency-DAG check).
//! Every node ships with table + property + determinism-hash tests and doc
//! comments that feed the generated catalog (DECISIONS.md).
//!
//! Stage 0 (doc 15): one hand-registered stub node (`add`) proves the
//! registry → `CATALOG.md` pipeline. Stage 1 replaces hand-rolled specs with
//! `#[node]` reflection; stage 4 brings the ~30 spike nodes.

use cicada_core::spec::NodeSpec;

pub mod maths;

/// Every node the stdlib registers, in stable catalog order.
///
/// Stage 1 replaces this hand-maintained list with compile-time registration
/// via `#[node]` (docs/08 §The node registry).
#[must_use]
pub fn registry() -> Vec<&'static NodeSpec> {
    vec![&maths::ADD_SPEC]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_are_unique() {
        let mut names: Vec<&str> = registry().iter().map(|spec| spec.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate node names in registry");
    }

    #[test]
    fn registry_categories_are_known() {
        for spec in registry() {
            assert!(
                cicada_core::catalog::CATEGORY_ORDER.contains(&spec.category),
                "node `{}` uses unknown category `{}` (docs/08 §Catalog)",
                spec.name,
                spec.category
            );
        }
    }
}
