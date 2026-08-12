//! Proc macros for the node ABI: `#[node]` and `#[derive(Ports)]`
//! (DECISIONS.md struct-in/struct-out row; docs/08 §The node registry).
//!
//! `#[derive(Ports)]` reflects struct fields into typed ports (a field with a
//! default is an optional port); `#[node]` assembles the `NodeSpec` — name,
//! title (doc comment first line), category, ports, purity, tier — and
//! registers it at compile time.
//!
//! Stage 0 (doc 15): empty. Both macros land in stage 1; until then the
//! stdlib hand-writes its `NodeSpec` statics.
