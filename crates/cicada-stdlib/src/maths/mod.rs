//! Maths & logic nodes (docs/08 §Catalog 3).
//!
//! Test standards per doc 14: table cases + a proptest property + a golden
//! blake3 determinism hash, for every stdlib node. Exact float `==` is
//! sanctioned in this category's tests because these nodes' contract IS
//! exact IEEE arithmetic (geometry tests use tolerance-aware asserts
//! instead). The transcendental nodes (trig, `ln`/`log`/`exp`) hash only
//! their IEEE-pinned special values (`sin 0 = +0`, `ln 1 = +0`, …) and
//! assert run-to-run identity elsewhere — platform libms differ in the
//! last ulp, so an irrational golden would be a false contract.

mod support;

pub mod absolute;
pub mod acos;
pub mod add;
pub mod and;
pub mod asin;
pub mod atan;
pub mod atan2;
pub mod average;
pub mod bounds;
pub mod ceiling;
pub mod construct_domain;
pub mod cos;
pub mod deconstruct_domain;
pub mod degrees;
pub mod divide;
pub mod equals;
pub mod exp;
pub mod floor;
pub mod larger;
pub mod ln;
pub mod log;
pub mod mass_addition;
pub mod max;
pub mod min;
pub mod modulo;
pub mod multiply;
pub mod negative;
pub mod not;
pub mod or;
pub mod pick;
pub mod power;
pub mod radians;
pub mod remap;
pub mod round;
pub mod sin;
pub mod smaller;
pub mod sqrt;
pub mod subtract;
pub mod tan;
pub mod xor;

pub use support::{AngleIn, BinaryIn, GateIn, ReduceIn, UnaryIn};
