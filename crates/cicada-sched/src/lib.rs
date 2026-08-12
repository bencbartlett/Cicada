//! The scheduler: solve generations with supersession, content-hash memo
//! stores (in-memory LRU + disk), rayon wavefront execution with per-element
//! fan-out, cancellation everywhere, cost models and progress/ETA (docs/12).
//!
//! The disk store lives in the user cache directory on the engine host —
//! NEVER inside the project folder by default; project dirs are cloud-synced
//! (DECISIONS.md cache row).
//!
//! Stage 0 (doc 15): empty. Scheduler-lite lands in stage 3 with
//! virtual-time fake-node tests.

pub use cicada_core as core;
