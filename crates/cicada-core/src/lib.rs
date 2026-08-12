//! Core value model for Cicada: content hashing, interning, axes, `Optional`
//! slots, and `ProjectConfig` (docs/08, docs/12, docs/14).
//!
//! Everything depends on this crate, so it stays tiny and fast (doc 14).
//!
//! Stage 0 (doc 15): only the node/port spec types and the catalog renderer
//! exist, wiring the registry → `docs/generated/CATALOG.md` pipeline end to
//! end. Stage 1 adds the value model and replaces hand-rolled specs with
//! `#[node]` / `#[derive(Ports)]` reflection.

pub mod catalog;
pub mod spec;
