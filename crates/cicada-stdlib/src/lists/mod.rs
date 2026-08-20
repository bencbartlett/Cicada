//! List & axis nodes (docs/08 §Catalog 4, docs/09 combinator inventory).
//! The wire-level combinators (`each()` map/zip) live in the dialect; these
//! are the node forms the spike needs. Element kinds flow through the `E`
//! type variable — `item` of a `[Point]` is a `Point`, `flatten` of a
//! `[[Point]]` is a `[Point]`, statically — and `E` carries element
//! optionality with it: every node here is slot-preserving (docs/08 rule
//! 6), so a `[Point?]` flows through as `[Point?]` and absent slots keep
//! their places. Elements leave a list only through `cull` and the
//! Optional-flow `compact`, which return an `IndexMap` so identity
//! survives. The strict-zip adapters (`pad_last`, `truncate`; `repeat` in
//! the sequences category) are the opt-in, visible forms of GH's
//! longest/shortest-list matching (docs/09).

#[cfg(test)]
pub(crate) mod support;

pub mod chunk;
pub mod compact;
pub mod concat;
pub mod cull;
pub mod dispatch;
pub mod duplicate;
pub mod flatten;
pub mod group_by;
pub mod item;
pub mod length;
pub mod nest;
pub mod pad_last;
pub mod partition;
pub mod reverse;
pub mod shift_list;
pub mod sort;
pub mod split_list;
pub mod transpose;
pub mod truncate;
