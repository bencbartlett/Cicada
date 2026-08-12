//! The engine server: axum app serving the protocol — JSON control plane,
//! generation-tagged binary geometry frames, sessions with a single-writer
//! lease, the op log, transport, and git integration (docs/13).
//!
//! Owns all authoritative state; the browser sends gesture-level intents and
//! receives authoritative deltas. axum/tokio are quarantined here (doc 14).
//! Nothing depends on this crate except `cicada-cli` (enforced by the
//! dependency-DAG check).
//!
//! Stage 0 (doc 15): empty. `cicada serve`, the protocol, and the debug
//! endpoints land in stage 5.

pub use cicada_core as core;
