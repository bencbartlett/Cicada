//! The `cicada` binary's implementation (doc 14): subcommand logic lives in
//! this library target so integration tests can drive the pieces directly;
//! `main.rs` is argument parsing only.
//!
//! Live today: `catalog` (stage 1) and `run` (stage 3 — the first
//! end-to-end surface). `serve`, `fmt`, `docs`, and `cache` land with
//! their stages.

pub mod catalog;
pub mod lower;
pub mod run;
pub mod scripts;
