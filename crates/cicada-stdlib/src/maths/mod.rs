//! Maths & logic nodes (docs/08 §Catalog 3).
//!
//! Test standards per doc 14: table cases + a proptest property + a golden
//! blake3 determinism hash, for every stdlib node. Exact float `==` is
//! sanctioned in this category's tests because these nodes' contract IS
//! exact IEEE arithmetic (geometry tests use tolerance-aware asserts
//! instead).

mod support;

pub mod add;
pub mod construct_domain;
pub mod deconstruct_domain;
pub mod divide;
pub mod modulo;
pub mod multiply;
pub mod power;
pub mod remap;
pub mod subtract;

pub use support::BinaryIn;
