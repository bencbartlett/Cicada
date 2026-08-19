//! The `cicada` binary's implementation (doc 14): subcommand logic lives in
//! this library target so integration tests can drive the pieces directly;
//! `main.rs` is argument parsing only.
//!
//! Live today: `catalog` (stage 1), `run` (stage 3 — the first end-to-end
//! surface), and `serve` (stage 5 — the app). `fmt`, `docs`, and `cache`
//! land with their stages. The lowering and script discovery that lived
//! here through stage 4 moved into `cicada-server` (its hydration path);
//! `run` is a printer over them.

pub mod catalog;
pub mod run;
pub mod serve;
