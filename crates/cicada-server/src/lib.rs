//! The engine server (docs/13): the axum app serving the protocol — JSON
//! control plane, generation-tagged binary geometry frames, sessions with a
//! single-writer lease, and the debug endpoints agents verify UI changes
//! with (doc 14). It owns ALL authoritative state; the browser sends
//! gesture-level intents and receives authoritative deltas.
//!
//! The pipeline hydration path lives here too (moved from `cicada-cli` at
//! stage 5): [`compile`] (parse → scripts → check → targets → gate),
//! [`lower`] (checked document → `SolveGraph`), [`scripts`] (Python script
//! nodes + the cancel bridge). `cicada run` drives them headlessly; the
//! [`session`] drives them live. axum/tokio are quarantined in this crate
//! (doc 14); nothing depends on it except `cicada-cli` (dependency-DAG
//! test).
//!
//! Stage-5 slice plus the v0.1 undo/redo (doc 17 item 1): one project
//! directory, one session per pipeline, single writer + read-only
//! observers, a snapshot op log with `undo` / `redo` / `batch` /
//! `apply_text` (+ `GET /api/edit/text`, `POST /api/edit/apply_text`); no
//! transport, no git panel yet; the byte-exact frame format is documented
//! in [`frames`] and docs/13.

mod atomic;
pub mod catalog;
pub mod compile;
pub mod display;
pub mod frames;
pub mod http;
pub mod layout;
pub mod lower;
pub mod protocol;
pub mod scripts;
pub mod session;
pub mod sidecar;
pub mod solve;
pub mod viewmodel;

pub use cicada_core as core;
pub use http::{DEFAULT_PORT, ServeConfig, ServeError, ServerHandle, serve};
