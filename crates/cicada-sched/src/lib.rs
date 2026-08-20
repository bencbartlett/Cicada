//! The scheduler (docs/12, stage 3 of doc 15): solve generations with
//! supersession, content-addressed memoization over a two-level disk store,
//! rayon wavefront execution with per-element `each()` fan-out in cost-sized
//! chunks, cancellation everywhere, latest-wins previews, and cost sampling.
//!
//! The one-line design (docs/12): **everything is content-addressed, so
//! cancellation and restart are nearly free, and "minimal recompute" is
//! just cache lookup.** Dirty cones are exact by construction — an edit
//! changes value hashes, changed hashes change `NodeKey`s, and unchanged
//! keys hit the memo table without executing anything.
//!
//! The disk store lives in the user cache directory on the engine host
//! ([`store::project_cache_dir`]) — NEVER inside the project folder by
//! default; project dirs are cloud-synced (DECISIONS.md cache row).
//!
//! Stage-3 slice, stated honestly: priorities (preview-first / critical
//! path), oversubscription control, associative decomposition, the
//! kernel-worker escape hatch, scrub warming, and the lock file arrive with
//! stages 4–5 (docs/12 specifies them; nothing here contradicts them).
//! Element-level persistence requires calibrated cost stats, so a cold
//! node's first fan persists at node granularity only (see `exec`).
//! Tests drive everything through fake nodes on **virtual time** (doc 14).

pub mod cancel;
pub mod clock;
pub mod cost;
pub mod exec;
pub mod graph;
pub mod key;
pub mod preview;
pub mod store;

pub use cicada_core as core;

pub use cancel::{CancelHook, CancelToken, NodeCtx};
pub use clock::{Clock, MonotonicClock, VirtualClock};
pub use cost::{CostSample, CostStats};
pub use exec::{
    Event, NodeFailure, NodeOutcome, NoopObserver, Observer, Scheduler, SchedulerConfig,
    SolveError, SolveReport, panic_message,
};
pub use graph::{GraphError, Input, NodeDecl, NodeError, NodeFn, NodeId, SolveGraph};
pub use key::{CACHE_EPOCH, KeyInputs, NodeKey, node_key};
pub use preview::{PreviewJob, PreviewSession};
pub use store::{
    BlobLocation, DiskStore, LogRecovery, MemoEntry, OpenReport, PACK_MAX_BYTES, StoreError,
    project_cache_dir,
};
