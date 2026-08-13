//! Core value model for Cicada (docs/08, docs/12, docs/14): content-hashed
//! immutable values, interning, Merkle lists with axes and `Optional` slots,
//! `ProjectConfig`, and the node/port specification types the registry,
//! catalog, checker, and canvas all share.
//!
//! Everything depends on this crate, so it stays tiny and fast (doc 14).
//!
//! Stage 1 (doc 15): value model + `#[node]`/`#[derive(Ports)]` reflection
//! (the macros live in `cicada-macros`; the traits and registration
//! machinery live here). Geometry kinds join from `cicada-geom` in stage 4.

pub mod catalog;
pub mod config;
pub mod hash;
pub mod intern;
pub mod scalar;
pub mod spatial;
pub mod spec;
pub mod value;
