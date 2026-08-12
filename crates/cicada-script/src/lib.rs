//! Script hosts: the wasmtime WASM runtime (epoch preemption = hard
//! cancellation; postcard buffers) and the Python worker subprocess pool
//! (length-prefixed `MessagePack` frames; kill = cancel) — doc 14 §The script
//! marshalling ABI.
//!
//! wasmtime and other heavy host dependencies are quarantined here (doc 14).
//! `unsafe` is permitted only inside FFI seam modules, each block with a
//! `// SAFETY:` comment.
//!
//! Stage 0 (doc 15): empty. The Python worker pool lands in stage 4; the
//! WASM host is explicitly out of the spike (v0.1+).

pub use cicada_core as core;
