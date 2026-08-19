//! Core value model for Cicada (docs/08, docs/12, docs/14): content-hashed
//! immutable values, interning, Merkle lists with axes and `Optional` slots,
//! `ProjectConfig`, and the node/port specification types the registry,
//! catalog, checker, and canvas all share.
//!
//! Everything depends on this crate, so it stays tiny and fast (doc 14).
//!
//! Stage 1 (doc 15): value model + `#[node]`/`#[derive(Ports)]` reflection
//! (the macros live in `cicada-macros`; the traits and registration
//! machinery live here). Stage 4 adds the geometry VALUE kinds
//! ([`geometry`]) — value types are core's by the dependency law;
//! constructive geometry lives in `cicada-geom`.

pub mod catalog;
pub mod config;
pub mod geometry;
pub mod hash;
pub mod intern;
pub mod marshal;
pub mod scalar;
pub mod spatial;
pub mod spec;
pub mod value;
