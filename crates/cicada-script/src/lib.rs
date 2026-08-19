//! Script hosts (docs/10 §5, docs/12): the Python worker pool (stage 4).
//! The WASM host (Rust script nodes, the v0.1 default) arrives later.
//!
//! Shape: a pool of persistent `CPython` subprocesses speaking length-framed
//! `MessagePack` over stdio (DECISIONS.md row 41 — `MessagePack` at the
//! Python boundary). The worker program is EMBEDDED in this crate
//! ([`WORKER_SOURCE`]) and needs no Python packages — scripts' own
//! dependencies (numpy, scipy) are their business. Cancellation is
//! `kill -9` on the worker (docs/12: user scripts are hard-cancellable by
//! construction); the pool respawns workers as needed, so a kill costs
//! one process start.

use cicada_core::hash::{KindTag, ValueHash, ValueHasher};

pub mod pool;
pub mod value;

pub use pool::{Described, KillSwitch, PortDesc, ScriptNodeDesc, WorkerPool};

/// The embedded Python worker program (protocol + `@cicada.node`
/// decorator + marshalling). Written to a temp file at pool start; its
/// bytes participate in nothing cache-relevant (script SOURCE files do,
/// via [`source_hash`]).
pub const WORKER_SOURCE: &str = include_str!("worker.py");

/// The cache-key hash of one script source (docs/12: script
/// `node_version` = "hash of the source file + compiler/toolchain
/// version" — `toolchain` is the interpreter's `sys.version`, reported by
/// [`pool::Described`]). A re-save with identical bytes recomputes
/// nothing; switching interpreters (or upgrading Python) recomputes
/// everything the scripts touched, exactly as the ledger demands.
#[must_use]
pub fn source_hash(source: &[u8], toolchain: &str) -> ValueHash {
    ValueHasher::new(KindTag::ScriptSource)
        .bytes(source)
        .bytes(toolchain.as_bytes())
        .finish()
}

/// Script-host failures. All loud; Python-side failures carry the
/// traceback tail so the red node names the actual line.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// No usable Python interpreter.
    #[error(
        "no Python interpreter found (tried {tried}); install Python 3 or set \
         CICADA_PYTHON to the interpreter path"
    )]
    NoPython {
        /// What was probed.
        tried: String,
    },
    /// Process/pipe I/O trouble.
    #[error("worker I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The worker's reply failed to decode.
    #[error("worker protocol: {0}")]
    Protocol(String),
    /// The script itself failed (import error, bad signature, exception) —
    /// the message carries the Python traceback tail.
    #[error("{0}")]
    Script(String),
    /// The call was cancelled: the worker was killed mid-flight.
    #[error("cancelled: the worker was killed")]
    Cancelled,
    /// A value refused to cross the boundary (unsupported kind, NaN back
    /// from Python, malformed wire value).
    #[error("marshalling: {0}")]
    Marshal(String),
}
