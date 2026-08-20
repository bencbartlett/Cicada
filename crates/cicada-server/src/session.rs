//! One pipeline = one session (docs/13 §Projects, pipelines, sessions):
//! the authoritative owner of the `.cic` text, its sidecar, the checked
//! and lowered graph, the solve loop, per-node statuses, the display set,
//! and the connected clients with their single-writer lease.
//!
//! Every canvas gesture arrives as an intent, becomes a minimal text edit
//! through `cicada_lang::writer` (docs/10 round-trip table), is persisted
//! immediately (no save button, docs/16), broadcast as an authoritative
//! delta, and solved after a ~30 ms structural debounce; `param_preview`
//! streams edit a scratch copy and go straight to the latest-wins loop.
//! Frames go out only for outputs whose value hash changed since the last
//! broadcast; a joining client gets the whole display set re-streamed.
//!
//! Undo/redo (docs/13 §Undo/redo, DECISIONS.md undo row revised
//! 2026-08-19): every successful write pushes one [`Op`] holding a **state
//! snapshot** (text + sidecar) before and after it onto the session's
//! [`OpLog`]; `undo`/`redo` restore a snapshot through the same persist +
//! delta path as any edit; `batch` applies several gestures as one op;
//! `apply_text` replaces whole files atomically against a base hash; an
//! external change (the watcher) is the reload barrier that clears the log.
//!
//! Locking discipline: `inner` (document/graph/clients) and `status` (the
//! per-node board, written from rayon worker threads) are separate mutexes;
//! neither is held while calling into the solve loop, and the solve loop
//! holds none of its own while calling back in — so no lock order exists
//! to violate.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use cicada_core::config::ProjectConfig;
use cicada_core::hash::ValueHash;
use cicada_core::spec::NodeSpec;
use cicada_lang::ast::Rhs;
use cicada_lang::check::BindingType;
use cicada_lang::diag::{Diagnostic, DiagnosticKind};
use cicada_lang::{Catalog, Document, Line, resolve, writer};
use cicada_sched::{
    CancelToken, Clock, DiskStore, Event, LogRecovery, MonotonicClock, NodeId, NodeOutcome,
    Observer, Scheduler, SchedulerConfig, SolveError, SolveReport, project_cache_dir,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::atomic::write_atomic;
use crate::compile::{self, Loaded};
use crate::display::{self, DisplayStats, PickTable};
use crate::lower::{Lowered, LoweredBinding, lower, lower_partial};
use crate::protocol::{
    Actor, ApplyTextRequest, ClientMessage, DeltaSource, HistoryView, LeaseView, NodeState,
    NodeStatus, ProbeCatalogEntry, ProbeVerdict, Role, ServerMessage, SolveSummary, ValueSummary,
    encode, is_gesture, is_write, type_tag,
};
use crate::scripts::ScriptCancel;
use crate::sidecar::Sidecar;
use crate::solve::{Job, JobKind, SolveLoop, SolveSink};
use crate::viewmodel::{self, GraphView, NodeRefs, WireEnd};

/// A path for humans: Windows' `\\?\` verbatim prefix stripped, forward
/// slashes.
#[must_use]
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('\\', "/")
}

/// Structural-edit debounce (docs/12 §Solve generations: ~30 ms).
pub const STRUCTURAL_DEBOUNCE: Duration = Duration::from_millis(30);
/// Status coalescing period (docs/13: ≤ 10 Hz).
pub const STATUS_PERIOD: Duration = Duration::from_millis(100);
/// Grid unit hint sent to clients (px per unit).
pub const UNIT_PX: u32 = 24;

/// Session construction options.
#[derive(Clone)]
pub struct SessionConfig {
    /// The project directory (display + relative paths).
    pub project_dir: PathBuf,
    /// The pipeline file (absolute).
    pub pipeline: PathBuf,
    /// Store override; default = the per-project user cache dir.
    pub cache_dir: Option<PathBuf>,
    /// Worker threads (0 = cores − 2).
    pub threads: usize,
    /// Project configuration (units, tolerance).
    pub project: ProjectConfig,
    /// The clock stamping op-log entries (`Op::at`, monotonic ms since the
    /// clock's epoch — never wall time). `None` = a monotonic clock anchored
    /// at open; tests inject a [`cicada_sched::VirtualClock`].
    pub op_clock: Option<Arc<dyn Clock>>,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("project_dir", &self.project_dir)
            .field("pipeline", &self.pipeline)
            .field("cache_dir", &self.cache_dir)
            .field("threads", &self.threads)
            .field("project", &self.project)
            .field(
                "op_clock",
                &self.op_clock.as_ref().map_or("monotonic", |_| "injected"),
            )
            .finish()
    }
}

/// How many ops the log keeps (the oldest drop off; git is the durable
/// history — docs/13).
pub const OP_LOG_CAP: usize = 200;

/// Session failures.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Reading the pipeline failed.
    #[error("reading {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// Writing the pipeline failed.
    #[error("writing {path}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// Script discovery failed.
    #[error(transparent)]
    Scripts(#[from] crate::scripts::ScriptsError),
    /// The sidecar is unreadable/invalid.
    #[error(transparent)]
    Sidecar(#[from] crate::sidecar::SidecarError),
    /// The store refused to open.
    #[error(transparent)]
    Store(#[from] cicada_sched::StoreError),
    /// The scheduler could not start.
    #[error(transparent)]
    Solve(#[from] SolveError),
    /// Graph assembly failed (a lowering bug).
    #[error(transparent)]
    Lower(#[from] crate::lower::LowerError),
    /// The pipeline is not inside the project directory — refused, never
    /// silently served from wherever it is.
    #[error("{pipeline} is not inside the project {project}")]
    OutsideProject {
        /// The pipeline.
        pipeline: PathBuf,
        /// The project directory.
        project: PathBuf,
    },
}

/// Why an intent was refused (sent back as an `error` message).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntentError {
    /// Read-only observer sent a write.
    #[error("read-only observer — take the lease to edit")]
    Lease,
    /// A writer gesture refused.
    #[error("{0}")]
    Writer(#[from] writer::WriterError),
    /// Unknown node/function/port.
    #[error("{0}")]
    Unknown(String),
    /// Malformed intent.
    #[error("{0}")]
    Protocol(String),
    /// Persisting failed.
    #[error("{0}")]
    Persist(String),
    /// The gesture is refused by the checker (blocked wire, cycle, missing
    /// port) — the text is never edited.
    #[error("{0}")]
    Refused(String),
    /// `undo` with nothing to undo (empty log, or cleared by the barrier).
    #[error("nothing to undo — {0}")]
    NothingToUndo(String),
    /// `redo` with nothing to redo.
    #[error("nothing to redo — {0}")]
    NothingToRedo(String),
    /// `apply_text`: the caller's base is not the current text.
    #[error(
        "stale base: the pipeline text changed since it was read (current text hash \
         {current_text_hash}) — re-read and rebase the edit"
    )]
    StaleBase {
        /// The hash the caller must base its next attempt on.
        current_text_hash: String,
    },
    /// `apply_text`: a submitted file does not parse (check diagnostics are
    /// allowed — red is a valid state; parse failures are not).
    #[error("{message}")]
    ParseError {
        /// What failed to parse.
        message: String,
        /// The parse-level diagnostics (doc-11 shape), when the `.cic` is
        /// the culprit.
        diagnostics: Vec<Diagnostic>,
    },
    /// `apply_text`: a path outside the allowed set (this pipeline, its
    /// sidecar, `scripts/*.py` next to it).
    #[error("{0}")]
    PathNotAllowed(String),
    /// `apply_text`: a file write failed; earlier files were restored.
    #[error("{0}")]
    Io(String),
    /// A `batch` element failed; the whole batch was rolled back.
    #[error("batch `{label}` failed at op {index} ({op}): {source}")]
    Batch {
        /// The batch label.
        label: String,
        /// The 0-based index of the failing op.
        index: usize,
        /// Its `type` tag.
        op: String,
        /// Why it failed.
        #[source]
        source: Box<IntentError>,
    },
}

impl IntentError {
    /// The machine `kind` of the `error` message. A batch failure carries
    /// the failing op's kind (the client reacts to that; `index` says
    /// where).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Lease => "lease",
            Self::Writer(_) => "writer",
            Self::Unknown(_) => "unknown",
            Self::Protocol(_) => "protocol",
            Self::Persist(_) => "persist",
            Self::Refused(_) => "refused",
            Self::NothingToUndo(_) => "nothing_to_undo",
            Self::NothingToRedo(_) => "nothing_to_redo",
            Self::StaleBase { .. } => "stale_base",
            Self::ParseError { .. } => "parse_error",
            Self::PathNotAllowed(_) => "path_not_allowed",
            Self::Io(_) => "io_error",
            Self::Batch { source, .. } => source.kind(),
        }
    }

    /// Kind-specific facts for the error payload (flattened).
    #[must_use]
    pub fn details(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        match self {
            Self::StaleBase { current_text_hash } => {
                map.insert(
                    "current_text_hash".to_owned(),
                    serde_json::Value::String(current_text_hash.clone()),
                );
            }
            Self::ParseError { diagnostics, .. } => {
                map.insert(
                    "diagnostics".to_owned(),
                    serde_json::to_value(diagnostics).unwrap_or_default(),
                );
            }
            Self::Batch { index, source, .. } => {
                map = source.details();
                map.insert("index".to_owned(), serde_json::Value::from(*index));
            }
            _ => {}
        }
        map
    }
}

/// The `{kind, message, …details}` JSON body of a refused intent (the WS
/// `error` payload minus the envelope; the HTTP routes' error body).
#[must_use]
pub fn error_body(error: &IntentError) -> serde_json::Value {
    let mut body = error.details();
    body.insert(
        "kind".to_owned(),
        serde_json::Value::String(error.kind().to_owned()),
    );
    body.insert(
        "message".to_owned(),
        serde_json::Value::String(error.to_string()),
    );
    serde_json::Value::Object(body)
}

/// A point in the undo/redo history: the pipeline text (the exact bytes
/// the writer emits) and the sidecar. Node refs are NOT part of it — they
/// are stable per name for the session's life (pick ids), whichever state
/// is current.
#[derive(Debug, Clone, PartialEq)]
struct StateSnapshot {
    text: String,
    sidecar: Sidecar,
}

/// One entry of the op log (docs/13 §Undo/redo).
#[derive(Debug, Clone)]
pub struct Op {
    /// Monotonic id (1-based, per session).
    pub id: u64,
    /// Human label (the delta's `source.label`).
    pub label: String,
    /// Who made it.
    pub actor: Actor,
    /// Server monotonic milliseconds (the session's op clock).
    pub at: u64,
    before: StateSnapshot,
    after: StateSnapshot,
}

/// The linear op log: `ops[..cursor]` are applied, `ops[cursor..]` is the
/// redo tail. Capped at [`OP_LOG_CAP`] (oldest dropped); a push truncates
/// the redo tail; the reload barrier clears everything.
#[derive(Debug, Default)]
struct OpLog {
    ops: VecDeque<Op>,
    cursor: usize,
    next_id: u64,
    /// The last clear was the reload barrier — `nothing_to_undo` says so.
    cleared_by_barrier: bool,
}

impl OpLog {
    fn push(
        &mut self,
        label: String,
        actor: Actor,
        before: StateSnapshot,
        after: StateSnapshot,
        at: u64,
    ) {
        self.ops.truncate(self.cursor);
        self.next_id += 1;
        self.ops.push_back(Op {
            id: self.next_id,
            label,
            actor,
            at,
            before,
            after,
        });
        while self.ops.len() > OP_LOG_CAP {
            self.ops.pop_front();
        }
        self.cursor = self.ops.len();
        self.cleared_by_barrier = false;
    }

    fn undo_target(&self) -> Option<&Op> {
        self.cursor.checked_sub(1).and_then(|i| self.ops.get(i))
    }

    fn redo_target(&self) -> Option<&Op> {
        self.ops.get(self.cursor)
    }

    /// Why there is nothing to undo/redo.
    fn empty_reason(&self, undo: bool) -> String {
        if self.cleared_by_barrier && self.ops.is_empty() {
            "the op log was cleared by a reload barrier (an external file change — git, an editor)"
                .to_owned()
        } else if undo {
            if self.ops.is_empty() {
                "no edits in this session yet".to_owned()
            } else {
                "every op is already undone".to_owned()
            }
        } else {
            "no undone op to redo".to_owned()
        }
    }

    fn clear_by_barrier(&mut self) {
        self.ops.clear();
        self.cursor = 0;
        self.cleared_by_barrier = true;
    }

    fn view(&self) -> HistoryView {
        HistoryView {
            can_undo: self.undo_target().is_some(),
            can_redo: self.redo_target().is_some(),
            undo_label: self.undo_target().map(|op| op.label.clone()),
            redo_label: self.redo_target().map(|op| op.label.clone()),
            depth: self.cursor,
        }
    }

    /// `/debug/state` → `ops`.
    fn debug_ops(&self) -> Vec<serde_json::Value> {
        self.ops
            .iter()
            .map(|op| {
                serde_json::json!({
                    "id": op.id,
                    "label": op.label,
                    "actor": op.actor,
                    "at": op.at,
                })
            })
            .collect()
    }
}

/// What a write gesture (or a batch of them) did to the in-memory state,
/// before the commit persists and broadcasts it.
#[derive(Debug, Default)]
struct Applied {
    /// The op's label.
    label: String,
    /// Bindings whose text changed (the delta's dirty set).
    dirty: Vec<String>,
    /// The `.cic` text was edited (else sidecar only: no solve).
    text_changed: bool,
    /// The display set may have changed (preview toggles): re-emit frames
    /// from the last complete generation after the commit.
    refresh_display: bool,
}

impl Applied {
    /// Fold a batch element into the batch's summary.
    fn absorb(&mut self, other: Applied) {
        for name in other.dirty {
            if !self.dirty.contains(&name) {
                self.dirty.push(name);
            }
        }
        self.text_changed |= other.text_changed;
        self.refresh_display |= other.refresh_display;
    }
}

/// How a commit touches the op log.
enum OpEffect {
    /// Push a new op (the `before` state → the committed state); truncates
    /// the redo tail. A commit that changed nothing (text and sidecar
    /// identical) pushes no op — there is nothing to undo.
    Push {
        /// Who made it.
        actor: Actor,
    },
    /// The caller already moved the cursor (undo/redo) — record nothing.
    Cursor,
}

/// Everything a write arm mutates in memory before its commit persists —
/// taken under the lock at the start of the arm, put back under the SAME
/// lock hold when anything fails (a gesture refused half-way, a persist
/// that could not land), so a refused edit never lingers for the next op
/// to broadcast and write. Graph, lowering and `text_hash` change only
/// after a successful persist and need no rollback.
struct RollbackPoint {
    document: Document,
    sidecar: Sidecar,
    refs: NodeRefs,
    cursor: usize,
}

impl RollbackPoint {
    fn capture(inner: &Inner) -> Self {
        Self {
            document: inner.loaded.document.clone(),
            sidecar: inner.sidecar.clone(),
            refs: inner.refs.clone(),
            cursor: inner.oplog.cursor,
        }
    }

    fn restore(self, inner: &mut Inner) {
        inner.loaded.document = self.document;
        inner.sidecar = self.sidecar;
        inner.refs = self.refs;
        inner.oplog.cursor = self.cursor;
        inner.loaded.recheck();
    }
}

/// A message to one client's socket.
#[derive(Debug, Clone)]
pub enum Outgoing {
    /// JSON control-plane text.
    Text(String),
    /// A binary frame.
    Binary(Bytes),
}

struct Client {
    tx: UnboundedSender<Outgoing>,
    role: Role,
    joined: u64,
}

/// One displayed output's last broadcast state.
#[derive(Debug, Clone)]
struct Displayed {
    hash: ValueHash,
    generation: u64,
    stats: DisplayStats,
}

/// A generation's report kept for the inspector.
struct Kept {
    generation: u64,
    lowered: Arc<Lowered>,
    report: Arc<SolveReport>,
}

struct Inner {
    loaded: Loaded,
    sidecar: Sidecar,
    lowered: Arc<Lowered>,
    graph: GraphView,
    seq: u64,
    refs: NodeRefs,
    picks: PickTable,
    clients: BTreeMap<u32, Client>,
    next_client: u32,
    join_counter: u64,
    writer: Option<u32>,
    display: HashMap<(u32, u32), Displayed>,
    last_complete: Option<Kept>,
    /// Pending screenshot requests: id → reply slot.
    screenshots: HashMap<u64, tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>>,
    next_screenshot: u64,
    /// blake3 of the text in memory (`loaded.document.emit()`) — ALWAYS;
    /// it is the base `apply_text` checks against and the hash
    /// `GET /api/edit/text` ships beside the text. After every successful
    /// persist memory == disk, so it is the file's hash too, and the
    /// watcher's echo guard is simply "disk hashes to this" (plus sidecar
    /// equality and the scripts fingerprint) — no separate memory of what
    /// was last written, which would mask a genuine external change back
    /// to a text this session once wrote.
    text_hash: [u8; 32],
    /// The undo/redo log.
    oplog: OpLog,
    /// blake3 of every `scripts/*.py` as loaded (file name → hash) — the
    /// watcher's echo guard for script writes (`apply_text`): a rescan
    /// whose files hash to exactly these is our own write, not a barrier.
    scripts_fingerprint: BTreeMap<String, [u8; 32]>,
}

/// The per-node status board — written by observer events on worker
/// threads, flushed to clients at ≤ 10 Hz and on generation boundaries.
struct StatusBoard {
    nodes: BTreeMap<String, NodeStatus>,
    changed: BTreeMap<String, NodeStatus>,
    summary: SolveSummary,
    started: Option<Instant>,
    /// The running generation's queue wait (job accepted → start).
    queued: Option<Duration>,
    /// Predicted nanos per pending node (cost-weighted ETA).
    predicted: HashMap<String, Option<u64>>,
    dirty: bool,
}

struct Core {
    config: SessionConfig,
    relative: String,
    inner: Mutex<Inner>,
    status: Mutex<StatusBoard>,
    scheduler: Arc<Scheduler>,
    scripts: Arc<ScriptCancel>,
    /// The debounce thread's shared state.
    debounce: Mutex<Option<Instant>>,
    debounce_wake: Condvar,
    shutdown: AtomicBool,
    /// Set once the loop exists (Core is built first). Weak: the loop's
    /// sink holds the Core, so a strong reference here would be a cycle
    /// that never drops the worker thread, scheduler, or store.
    solve: Mutex<Option<std::sync::Weak<SolveLoop>>>,
    threads: usize,
    notices: Mutex<Vec<String>>,
    /// Stamps `Op::at` (monotonic ms; injectable).
    op_clock: Arc<dyn Clock>,
    /// Session start (timings are relative to it).
    epoch: Instant,
    /// The last generations' timings (docs/15 measurement protocol: the
    /// preview-latency currency, read from `/debug/state`).
    timings: Mutex<std::collections::VecDeque<GenerationTiming>>,
}

/// One generation's timing record (docs/15 measurement protocol; read
/// from `/debug/state` → `timings`). Preview latency on the server side is
/// `queued_ms + elapsed_ms`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GenerationTiming {
    /// The generation.
    pub generation: u64,
    /// `structural` / `preview` / `explicit`.
    pub kind: &'static str,
    /// Milliseconds since the session opened when it started.
    pub started_ms: f64,
    /// Wall milliseconds the job waited before starting: from the session
    /// accepting the work (a `param_preview` intent's arrival, before its
    /// lowering; the structural debounce firing; the load) to the
    /// generation's start. `0` for explicit runs (they never queue).
    pub queued_ms: f64,
    /// Wall milliseconds from start to completion (frames included).
    pub elapsed_ms: Option<f64>,
    /// Ended cancelled.
    pub cancelled: bool,
    /// Present only when an explicit cancel (Esc) ended this generation:
    /// server-side wall milliseconds from the first `cancel()` call to the
    /// solve loop flipping idle (completion hooks and frame emission
    /// included) — the docs/15 "Esc always works" currency, measured where
    /// the client's poll cannot blur it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_to_idle_ms: Option<f64>,
    /// Nodes computed / cached.
    pub computed: usize,
    /// Cache hits.
    pub cached: usize,
    /// Frame bytes broadcast for it.
    pub frame_bytes: usize,
}

/// How many timing records to keep: a 5 s slider stream at 60 Hz is 300
/// generations (more when the cone is cheaper than the tick), and the
/// harness reads them all back afterwards — the ring must hold a whole
/// measurement with room to spare.
pub const TIMINGS_KEPT: usize = 1024;

/// A live pipeline session.
pub struct Session {
    core: Arc<Core>,
    solve: Arc<SolveLoop>,
    debouncer: Option<std::thread::JoinHandle<()>>,
    ticker: Option<std::thread::JoinHandle<()>>,
}

/// The session's writes held off ([`Session::hold_writes`]): the document
/// lock, owned by the caller until [`Session::reload_from_disk_held`]
/// consumes it (or it is dropped — on the error path of whatever the
/// caller was doing to the files, which then simply never reloads).
#[must_use = "dropping the hold lets writes through again; pass it to reload_from_disk_held"]
pub struct WriteHold<'a>(std::sync::MutexGuard<'a, Inner>);

impl std::fmt::Debug for WriteHold<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteHold")
    }
}

impl Session {
    /// Open a session: read + check + lower the pipeline, open the store,
    /// start the scheduler and the solve loop, run the first generation.
    ///
    /// # Errors
    ///
    /// [`SessionError`].
    #[allow(clippy::too_many_lines)] // construction wires every part exactly once
    pub fn open(config: SessionConfig) -> Result<Arc<Self>, SessionError> {
        let text =
            std::fs::read_to_string(&config.pipeline).map_err(|source| SessionError::Read {
                path: config.pipeline.clone(),
                source,
            })?;
        let scripts_cancel = ScriptCancel::new();
        let loaded = compile::load(&config.pipeline, &text, &scripts_cancel)?;
        let sidecar = Sidecar::load(&Sidecar::path_for(&config.pipeline))?;
        let lowered = Arc::new(lower_partial(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &config.project,
            &loaded.scripts,
        )?);
        let mut refs = NodeRefs::default();
        let graph = viewmodel::build(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            &lowered,
            &sidecar,
            &mut refs,
        );

        let store_dir = match &config.cache_dir {
            Some(dir) => dir.clone(),
            None => project_cache_dir(&config.pipeline)?,
        };
        let (store, open_report) = DiskStore::open(&store_dir)?;
        let mut notices = Vec::new();
        match open_report.recovery {
            None => {}
            Some(LogRecovery::TornTail) => notices.push(
                "memo log ended in a torn record (crash mid-write?); truncated there — completed \
                 work before it was kept"
                    .to_owned(),
            ),
            Some(LogRecovery::CorruptRecord {
                offset,
                bytes_dropped,
            }) => notices.push(format!(
                "memo log had an undecodable record at byte {offset}; {bytes_dropped} bytes of \
                 later cached work were dropped and will recompute"
            )),
        }
        match open_report.pack_recovery {
            None => {}
            Some(LogRecovery::TornTail) => notices.push(
                "value pack ended in a torn frame (crash mid-write?); truncated there — the \
                 values before it were kept, the torn one recomputes"
                    .to_owned(),
            ),
            Some(LogRecovery::CorruptRecord {
                offset,
                bytes_dropped,
            }) => notices.push(format!(
                "value pack had an unframeable record at byte {offset}; {bytes_dropped} bytes of \
                 later cached values were dropped and will recompute"
            )),
        }
        let scheduler = Arc::new(Scheduler::new(
            Arc::new(store),
            Arc::new(MonotonicClock::new()),
            SchedulerConfig {
                threads: config.threads,
                ..SchedulerConfig::default()
            },
        )?);
        let threads = scheduler.threads();
        let relative = config
            .pipeline
            .strip_prefix(&config.project_dir)
            .map_err(|_| SessionError::OutsideProject {
                pipeline: config.pipeline.clone(),
                project: config.project_dir.clone(),
            })?
            .to_string_lossy()
            .replace('\\', "/");
        let text_hash = *blake3::hash(text.as_bytes()).as_bytes();
        let scripts_fingerprint = scripts_fingerprint(&config.pipeline)?;
        let op_clock: Arc<dyn Clock> = match config.op_clock.clone() {
            Some(clock) => clock,
            None => Arc::new(MonotonicClock::new()),
        };
        let core = Arc::new(Core {
            relative,
            inner: Mutex::new(Inner {
                loaded,
                sidecar,
                lowered,
                graph,
                seq: 0,
                refs,
                picks: PickTable::default(),
                clients: BTreeMap::new(),
                next_client: 0,
                join_counter: 0,
                writer: None,
                display: HashMap::new(),
                last_complete: None,
                screenshots: HashMap::new(),
                next_screenshot: 0,
                text_hash,
                oplog: OpLog::default(),
                scripts_fingerprint,
            }),
            status: Mutex::new(StatusBoard {
                nodes: BTreeMap::new(),
                changed: BTreeMap::new(),
                summary: SolveSummary::default(),
                started: None,
                queued: None,
                predicted: HashMap::new(),
                dirty: false,
            }),
            scheduler: Arc::clone(&scheduler),
            scripts: Arc::clone(&scripts_cancel),
            debounce: Mutex::new(None),
            debounce_wake: Condvar::new(),
            shutdown: AtomicBool::new(false),
            solve: Mutex::new(None),
            threads,
            notices: Mutex::new(notices),
            op_clock,
            epoch: Instant::now(),
            timings: Mutex::new(std::collections::VecDeque::with_capacity(TIMINGS_KEPT)),
            config,
        });
        let sink: Arc<dyn SolveSink> = core.clone();
        let solve = Arc::new(SolveLoop::new(scheduler, scripts_cancel, sink));
        *core
            .solve
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::downgrade(&solve));
        core.seed_statuses();

        // The debounce thread: sleeps until the deadline, then submits.
        let debounce_core = Arc::clone(&core);
        let debouncer = std::thread::Builder::new()
            .name("cicada-debounce".to_owned())
            .spawn(move || debounce_loop(&debounce_core))
            .ok();
        // The status ticker: flushes coalesced statuses at ≤ 10 Hz.
        let ticker_core = Arc::clone(&core);
        let ticker = std::thread::Builder::new()
            .name("cicada-status".to_owned())
            .spawn(move || {
                while !ticker_core.shutdown.load(Ordering::SeqCst) {
                    std::thread::sleep(STATUS_PERIOD);
                    ticker_core.flush_status(false);
                }
            })
            .ok();
        let session = Arc::new(Self {
            core,
            solve,
            debouncer,
            ticker,
        });
        session.submit_structural_now();
        Ok(session)
    }

    /// The pipeline path relative to the project.
    #[must_use]
    pub fn relative(&self) -> &str {
        &self.core.relative
    }

    /// The pipeline's absolute path.
    #[must_use]
    pub fn pipeline(&self) -> &Path {
        &self.core.config.pipeline
    }

    /// The store directory.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.core.scheduler.store().root().to_owned()
    }

    // --------------------------------------------------------- clients --

    /// Register a client socket. The first client takes the write lease;
    /// later ones observe. Returns `(client id, role)`. The caller then
    /// sends [`Self::hello`], [`Self::snapshot`], and
    /// [`Self::restream_display`].
    #[must_use]
    pub fn connect(&self, tx: UnboundedSender<Outgoing>) -> (u32, Role) {
        let mut inner = self.core.lock_inner();
        inner.next_client += 1;
        inner.join_counter += 1;
        let id = inner.next_client;
        let role = if inner.writer.is_none() {
            inner.writer = Some(id);
            Role::Writer
        } else {
            Role::Observer
        };
        let joined = inner.join_counter;
        inner.clients.insert(id, Client { tx, role, joined });
        let lease = lease_view(&inner);
        // Everyone learns the new roster.
        for (&other, client) in &inner.clients {
            if other != id {
                let _ = client.tx.send(Outgoing::Text(encode(
                    inner.seq,
                    &ServerMessage::Lease {
                        lease: lease.clone(),
                        role: client.role,
                    },
                )));
            }
        }
        (id, role)
    }

    /// A client left. A departing writer hands the lease to the oldest
    /// observer after the caller's grace period (`transfer_lease_if_free`).
    pub fn disconnect(&self, id: u32) {
        let mut inner = self.core.lock_inner();
        inner.clients.remove(&id);
        if inner.writer == Some(id) {
            inner.writer = None;
        }
        let lease = lease_view(&inner);
        for client in inner.clients.values() {
            let _ = client.tx.send(Outgoing::Text(encode(
                inner.seq,
                &ServerMessage::Lease {
                    lease: lease.clone(),
                    role: client.role,
                },
            )));
        }
    }

    /// After the writer's grace period: if no writer, promote the oldest
    /// observer (docs/13: automatic transfer, 5 s grace).
    pub fn transfer_lease_if_free(&self) {
        let mut inner = self.core.lock_inner();
        if inner.writer.is_some() {
            return;
        }
        let Some((&oldest, _)) = inner.clients.iter().min_by_key(|(_, c)| c.joined) else {
            return;
        };
        inner.writer = Some(oldest);
        if let Some(client) = inner.clients.get_mut(&oldest) {
            client.role = Role::Writer;
        }
        broadcast_lease(&inner);
    }

    /// Does `id` hold the write lease?
    #[must_use]
    pub fn is_writer(&self, id: u32) -> bool {
        self.core.lock_inner().writer == Some(id)
    }

    /// Is any client connected?
    #[must_use]
    pub fn has_clients(&self) -> bool {
        !self.core.lock_inner().clients.is_empty()
    }

    /// The `hello` message for a client.
    #[must_use]
    pub fn hello(&self, id: u32, role: Role) -> String {
        let inner = self.core.lock_inner();
        encode(
            inner.seq,
            &ServerMessage::Hello {
                client_id: id,
                role,
                protocol: crate::protocol::PROTOCOL_VERSION,
                engine: format!("cicada {}", env!("CARGO_PKG_VERSION")),
                project: display_path(&self.core.config.project_dir),
                pipeline: self.core.relative.clone(),
                unit_px: UNIT_PX,
            },
        )
    }

    /// The full snapshot (initial load / resync / reload barrier).
    #[must_use]
    pub fn snapshot(&self, barrier: bool, reason: &str) -> String {
        let inner = self.core.lock_inner();
        self.core.snapshot_locked(&inner, barrier, reason)
    }

    /// The current pipeline text and its hash — `GET /api/edit/text`, the
    /// base an agent reads before an `apply_text`.
    #[must_use]
    pub fn edit_text(&self) -> serde_json::Value {
        let inner = self.core.lock_inner();
        serde_json::json!({
            "path": self.core.relative,
            "text": inner.loaded.document.emit(),
            "text_hash": hex(&inner.text_hash),
        })
    }

    /// The undo/redo state (tests, `/debug/state`).
    #[must_use]
    pub fn history(&self) -> HistoryView {
        self.core.lock_inner().oplog.view()
    }

    /// Re-stream every displayed output's frames to ONE client (join /
    /// `resync_display`).
    pub fn restream_display(&self, id: u32) {
        let mut inner = self.core.lock_inner();
        let Some(client) = inner.clients.get(&id) else {
            return;
        };
        let tx = client.tx.clone();
        let generation = inner
            .display
            .values()
            .map(|d| d.generation)
            .max()
            .unwrap_or(0);
        let _ = tx.send(Outgoing::Text(encode(
            inner.seq,
            &ServerMessage::DisplayReset { generation },
        )));
        let store = Arc::clone(self.core.scheduler.store());
        let tolerance = self.core.config.project.tol();
        let keys: Vec<((u32, u32), Displayed)> =
            inner.display.iter().map(|(k, v)| (*k, v.clone())).collect();
        for ((node, output), displayed) in keys {
            match store.load_value(&displayed.hash) {
                Ok(value) => {
                    let frames = display::frames_for_value(
                        &value,
                        displayed.generation,
                        node,
                        output,
                        &mut inner.picks,
                        tolerance,
                    );
                    for frame in frames.frames {
                        let _ = tx.send(Outgoing::Binary(Bytes::from(frame)));
                    }
                }
                Err(error) => {
                    let _ = tx.send(Outgoing::Text(encode(
                        inner.seq,
                        &ServerMessage::Notice {
                            level: "warning".to_owned(),
                            message: format!(
                                "display value {} could not be reloaded from the store: {error}",
                                displayed.hash
                            ),
                        },
                    )));
                }
            }
        }
        for notice in self.core.take_notices() {
            let _ = tx.send(Outgoing::Text(encode(
                inner.seq,
                &ServerMessage::Notice {
                    level: "warning".to_owned(),
                    message: notice,
                },
            )));
        }
    }

    // --------------------------------------------------------- intents --

    /// Handle one intent from `client`. Errors are sent back to that
    /// client as `error` messages; nothing here panics on bad input. Every
    /// write arm rolls its own mutations back under its own lock hold
    /// ([`write_or_roll_back`]) — nothing is left for an outer pass to undo
    /// after the lock was released.
    pub fn handle(&self, client: u32, intent_id: Option<String>, message: ClientMessage) {
        let result = self.dispatch(client, intent_id.clone(), message);
        if let Err(error) = result {
            let inner = self.core.lock_inner();
            if let Some(c) = inner.clients.get(&client) {
                let _ = c.tx.send(Outgoing::Text(encode(
                    inner.seq,
                    &ServerMessage::Error {
                        intent_id,
                        kind: error.kind().to_owned(),
                        message: error.to_string(),
                        details: error.details(),
                    },
                )));
            }
        }
    }

    #[allow(clippy::too_many_lines)] // the intent table, one arm per message
    fn dispatch(
        &self,
        client: u32,
        intent_id: Option<String>,
        message: ClientMessage,
    ) -> Result<(), IntentError> {
        {
            let inner = self.core.lock_inner();
            if is_write(&message) && inner.writer != Some(client) {
                return Err(IntentError::Lease);
            }
        }
        let source = |label: String| DeltaSource {
            client: Some(client),
            intent_id: intent_id.clone(),
            label,
        };
        match message {
            // A canvas write gesture: edit in place, then ONE commit that
            // persists, records the op, and broadcasts the delta.
            gesture if is_gesture(&gesture) => {
                let mut inner = self.core.lock_inner();
                write_or_roll_back(&mut inner, |inner| {
                    let before = state_snapshot(inner);
                    let applied = Self::apply_gesture(inner, gesture)?;
                    let source = source(applied.label.clone());
                    self.commit(
                        inner,
                        source,
                        applied,
                        before,
                        OpEffect::Push {
                            actor: Actor::Human,
                        },
                    )
                })
            }
            ClientMessage::Hello { .. } => Ok(()),
            ClientMessage::ParamPreview { node, port, value } => {
                // The queue clock starts HERE — the lowering below is part
                // of what the user waits for after moving the slider.
                let accepted = Instant::now();
                let job = {
                    let inner = self.core.lock_inner();
                    let mut scratch = inner.loaded.document.clone();
                    apply_param(&mut scratch, &node, port.as_deref(), &value)?;
                    let resolution = resolve(&scratch, &Catalog::new(&inner.loaded.specs));
                    let lowered = lower_partial(
                        &scratch,
                        &resolution,
                        &inner.loaded.specs,
                        &self.core.config.project,
                        &inner.loaded.scripts,
                    )
                    .map_err(|e| IntentError::Protocol(e.to_string()))?;
                    let targets = all_targets(&lowered);
                    Job {
                        lowered: Arc::new(lowered),
                        targets,
                        kind: JobKind::Preview,
                        submitted: accepted,
                    }
                };
                self.solve.submit(job);
                Ok(())
            }
            ClientMessage::Undo {} => {
                let mut inner = self.core.lock_inner();
                let Some(op) = inner.oplog.undo_target() else {
                    return Err(IntentError::NothingToUndo(inner.oplog.empty_reason(true)));
                };
                let label = format!("undo: {}", op.label);
                let target = op.before.clone();
                write_or_roll_back(&mut inner, |inner| {
                    let before = state_snapshot(inner);
                    let applied = restore_state(inner, &target, label);
                    inner.oplog.cursor -= 1;
                    let source = source(applied.label.clone());
                    self.commit(inner, source, applied, before, OpEffect::Cursor)
                })
            }
            ClientMessage::Redo {} => {
                let mut inner = self.core.lock_inner();
                let Some(op) = inner.oplog.redo_target() else {
                    return Err(IntentError::NothingToRedo(inner.oplog.empty_reason(false)));
                };
                let label = format!("redo: {}", op.label);
                let target = op.after.clone();
                write_or_roll_back(&mut inner, |inner| {
                    let before = state_snapshot(inner);
                    let applied = restore_state(inner, &target, label);
                    inner.oplog.cursor += 1;
                    let source = source(applied.label.clone());
                    self.commit(inner, source, applied, before, OpEffect::Cursor)
                })
            }
            ClientMessage::Batch { ops, label } => {
                if ops.is_empty() {
                    return Err(IntentError::Protocol(format!("batch `{label}` has no ops")));
                }
                // Validate the whole list BEFORE touching anything: a batch
                // is gestures only (no previews, cancels, nested batches,
                // undo/redo, apply_text).
                if let Some((index, bad)) = ops.iter().enumerate().find(|(_, op)| !is_gesture(op)) {
                    return Err(IntentError::Batch {
                        label,
                        index,
                        op: type_tag(bad),
                        source: Box::new(IntentError::Protocol(
                            "not a canvas write gesture — a batch holds place_node / connect / \
                             disconnect / accept_lift / set_param / rename / delete_node / \
                             toggle_disable / move_node / set_preview only"
                                .to_owned(),
                        )),
                    });
                }
                let mut inner = self.core.lock_inner();
                // All or nothing: an element that fails (or a persist that
                // cannot land) puts the pre-batch state back — memory by
                // the rollback, disk by the commit.
                write_or_roll_back(&mut inner, |inner| {
                    let before = state_snapshot(inner);
                    let mut summary = Applied {
                        label: label.clone(),
                        ..Applied::default()
                    };
                    for (index, op) in ops.into_iter().enumerate() {
                        let tag = type_tag(&op);
                        match Self::apply_gesture(inner, op) {
                            Ok(applied) => {
                                if applied.text_changed {
                                    // Later elements see the earlier ones
                                    // (a connect after a place needs the
                                    // new binding resolved).
                                    inner.loaded.recheck();
                                }
                                summary.absorb(applied);
                            }
                            Err(source) => {
                                return Err(IntentError::Batch {
                                    label,
                                    index,
                                    op: tag,
                                    source: Box::new(source),
                                });
                            }
                        }
                    }
                    let source = source(label);
                    self.commit(
                        inner,
                        source,
                        summary,
                        before,
                        OpEffect::Push {
                            actor: Actor::Human,
                        },
                    )
                })
            }
            ClientMessage::ApplyText(request) => {
                let source = source(request.label.clone());
                self.apply_text(&request, source).map(|_| ())
            }
            ClientMessage::Cancel {} => {
                self.cancel();
                Ok(())
            }
            ClientMessage::Inspect { node } => {
                let inner = self.core.lock_inner();
                let (generation, outputs) = self.core.node_values(&inner, &node);
                send_to(
                    &inner,
                    client,
                    &ServerMessage::NodeValues {
                        node,
                        outputs,
                        generation,
                    },
                );
                Ok(())
            }
            ClientMessage::InspectWire { to } => {
                let inner = self.core.lock_inner();
                let Some(wire) = inner.graph.wires.iter().find(|w| w.to == to).cloned() else {
                    return Err(IntentError::Unknown(format!(
                        "no wire into {}.{}",
                        to.node, to.port
                    )));
                };
                let (_, outputs) = self.core.node_values(&inner, &wire.from.node);
                let summary = outputs
                    .into_iter()
                    .find(|(port, _)| *port == wire.from.port)
                    .and_then(|(_, summary)| summary);
                let pairing = match (wire.lift, summary.as_ref().and_then(|s| s.count)) {
                    (0, _) => "direct (no iteration)".to_owned(),
                    (depth, Some(count)) => format!("map ×{depth} over {count} elements"),
                    (depth, None) => format!("map ×{depth}"),
                };
                send_to(
                    &inner,
                    client,
                    &ServerMessage::WireValues {
                        to,
                        from: wire.from,
                        summary,
                        pairing,
                    },
                );
                Ok(())
            }
            ClientMessage::ProbeWire { from } => {
                let inner = self.core.lock_inner();
                let (targets, catalog) = probe_wire(&inner, &from)?;
                send_to(
                    &inner,
                    client,
                    &ServerMessage::WireProbe {
                        intent_id,
                        from,
                        targets,
                        catalog,
                    },
                );
                Ok(())
            }
            ClientMessage::ResyncDisplay {} => {
                self.restream_display(client);
                Ok(())
            }
            ClientMessage::TakeLease {} => {
                let mut inner = self.core.lock_inner();
                let previous = inner.writer;
                inner.writer = Some(client);
                if let Some(prev) = previous
                    && let Some(c) = inner.clients.get_mut(&prev)
                {
                    c.role = Role::Observer;
                }
                if let Some(c) = inner.clients.get_mut(&client) {
                    c.role = Role::Writer;
                }
                broadcast_lease(&inner);
                Ok(())
            }
            ClientMessage::Screenshot {
                id,
                png_base64,
                error,
            } => {
                let mut inner = self.core.lock_inner();
                if let Some(slot) = inner.screenshots.remove(&id) {
                    let result = match (png_base64, error) {
                        (Some(b64), _) => {
                            use base64::Engine as _;
                            base64::engine::general_purpose::STANDARD
                                .decode(b64.as_bytes())
                                .map_err(|e| format!("bad base64: {e}"))
                        }
                        (None, Some(error)) => Err(error),
                        (None, None) => Err("client sent no image".to_owned()),
                    };
                    let _ = slot.send(result);
                }
                Ok(())
            }
            // Every gesture is caught by the guard arm above; this arm
            // exists for the exhaustiveness check only.
            other => Err(IntentError::Protocol(format!(
                "`{}` is not a handled intent — bug",
                type_tag(&other)
            ))),
        }
    }

    /// Apply ONE canvas write gesture to the in-memory document/sidecar
    /// (docs/10 round-trip table) — nothing persisted, nothing broadcast;
    /// the caller commits. Shared by the single-gesture path and `batch`.
    #[allow(clippy::too_many_lines)] // the gesture table, one arm per gesture
    fn apply_gesture(inner: &mut Inner, message: ClientMessage) -> Result<Applied, IntentError> {
        match message {
            ClientMessage::PlaceNode {
                func,
                cell,
                connect,
            } => {
                if !inner.loaded.specs.iter().any(|spec| spec.name == func) {
                    return Err(IntentError::Unknown(format!(
                        "no node named `{func}` in the catalog"
                    )));
                }
                let deps: Vec<&str> = connect
                    .as_ref()
                    .map(|c| vec![c.from.node.as_str()])
                    .unwrap_or_default();
                let name = writer::place(&mut inner.loaded.document, &func, &deps)?;
                if let Some(cell) = cell {
                    inner.sidecar.set_cell(&name, Some(cell));
                }
                if let Some(spec) = connect {
                    connect_checked(inner, &spec.from, &name, &spec.to_port, spec.lift)?;
                }
                Ok(Applied {
                    label: format!("place {func}"),
                    dirty: vec![name],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::Connect { from, to, lift } => {
                connect_checked(inner, &from, &to.node, &to.port, lift)?;
                Ok(Applied {
                    label: format!("wire {}.{} → {}.{}", from.node, from.port, to.node, to.port),
                    dirty: vec![to.node],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::Disconnect { to } => {
                writer::remove_kwarg(&mut inner.loaded.document, &to.node, &to.port)?;
                Ok(Applied {
                    label: format!("unwire {}.{}", to.node, to.port),
                    dirty: vec![to.node],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::AcceptLift { node, port } => {
                writer::wrap_each(&mut inner.loaded.document, &node, &port)?;
                Ok(Applied {
                    label: format!("lift {node}.{port}"),
                    dirty: vec![node],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::SetParam { node, port, value } => {
                apply_param(&mut inner.loaded.document, &node, port.as_deref(), &value)?;
                let label = match &port {
                    Some(port) => format!("set {node}.{port} = {value}"),
                    None => format!("set {node} = {value}"),
                };
                Ok(Applied {
                    label,
                    dirty: vec![node],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::Rename { node, new } => {
                if inner.loaded.specs.iter().any(|spec| spec.name == new) {
                    // docs/10 §5: a binding named like a callable would
                    // shadow the node for later calls.
                    return Err(IntentError::Refused(format!(
                        "`{new}` is a node name in the catalog — a binding cannot take it"
                    )));
                }
                writer::rename(&mut inner.loaded.document, &node, &new)?;
                inner.sidecar.rename(&node, &new);
                inner.refs.rename(&node, &new);
                Ok(Applied {
                    label: format!("rename {node} → {new}"),
                    dirty: vec![new],
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::DeleteNode { node } => {
                // Downstream references become red — the dirty set names
                // them so the client can flash the reds.
                let dependents = dependents_of(&inner.loaded.document, &node);
                writer::delete(&mut inner.loaded.document, &node)?;
                inner.sidecar.remove(&node);
                inner.sidecar.remove_from_groups(&node);
                Ok(Applied {
                    label: format!("delete {node}"),
                    dirty: dependents,
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::ToggleDisable { node } => {
                // Downstream goes red ("disabled") or green again (a cache
                // hit, usually): the dirty set names the node and them.
                let mut dirty = vec![node.clone()];
                dirty.extend(dependents_of(&inner.loaded.document, &node));
                let state = writer::toggle_disable(&mut inner.loaded.document, &node)?;
                let verb = match state {
                    writer::DisableState::Disabled => "disable",
                    writer::DisableState::Enabled => "enable",
                };
                Ok(Applied {
                    label: format!("{verb} {node}"),
                    dirty,
                    text_changed: true,
                    refresh_display: false,
                })
            }
            ClientMessage::MoveNode { node, cell } => {
                // The graph view lags the document inside a batch (a node
                // placed two ops ago is not rebuilt yet): the document is
                // the authority, the view covers `#off`/broken lines.
                if inner.graph.node(&node).is_none()
                    && inner.loaded.document.find_binding(&node).is_none()
                {
                    return Err(IntentError::Unknown(format!("no node named `{node}`")));
                }
                inner.sidecar.set_cell(&node, cell);
                Ok(Applied {
                    label: format!("move {node}"),
                    dirty: Vec::new(),
                    text_changed: false,
                    refresh_display: false,
                })
            }
            ClientMessage::SetPreview { node, on } => {
                let view = inner.graph.node(&node);
                if view.is_none() && inner.loaded.document.find_binding(&node).is_none() {
                    return Err(IntentError::Unknown(format!("no node named `{node}`")));
                }
                // An override equal to the default is no override (the
                // sidecar stays near-empty by construction). A node the
                // view does not know yet (placed earlier in this batch)
                // keeps the override as given.
                let on = match view {
                    Some(view) => {
                        let default_on = view.outputs.iter().any(|o| o.displayable);
                        on.filter(|&flag| flag != default_on)
                    }
                    None => on,
                };
                inner.sidecar.set_preview(&node, on);
                Ok(Applied {
                    label: format!("preview {node}"),
                    dirty: Vec::new(),
                    text_changed: false,
                    refresh_display: true,
                })
            }
            other => Err(IntentError::Protocol(format!(
                "`{}` is not a canvas write gesture",
                type_tag(&other)
            ))),
        }
    }

    /// Persist text (when it changed) + sidecar after a write, recheck,
    /// relower, rebuild the view, bump `seq`, record the op, broadcast the
    /// delta, and schedule the structural solve (text changes) or re-emit
    /// frames (preview toggles). THE one path every edit — gesture, batch,
    /// undo, redo — takes to disk and to the clients.
    ///
    /// `before` is the state the disk holds as the call starts (memory ==
    /// disk after every successful commit). A persist that fails half-way
    /// — the text landed, the sidecar could not (a transient lock on a
    /// synced project dir) — takes the text off the disk again, so the
    /// disk never keeps a refused edit and `text_hash` stays the hash of
    /// the text in memory (the caller rolls memory back). A commit that
    /// changed nothing pushes no op.
    fn commit(
        &self,
        inner: &mut Inner,
        source: DeltaSource,
        applied: Applied,
        before: StateSnapshot,
        effect: OpEffect,
    ) -> Result<(), IntentError> {
        let text = inner.loaded.document.emit();
        self.persist(inner, &text, applied.text_changed, &before)?;
        if applied.text_changed {
            inner.loaded.recheck();
        }
        self.core.rebuild(inner);
        inner.seq += 1;
        if let OpEffect::Push { actor } = effect {
            let after = StateSnapshot {
                text: text.clone(),
                sidecar: inner.sidecar.clone(),
            };
            if after != before {
                let at = self.core.now_ms();
                inner
                    .oplog
                    .push(source.label.clone(), actor, before, after, at);
            }
        }
        let message = ServerMessage::Delta {
            source,
            graph: inner.graph.clone(),
            text,
            dirty: applied.dirty,
            history: inner.oplog.view(),
        };
        broadcast(inner, &message);
        if applied.text_changed {
            self.core.seed_statuses_locked(inner);
            self.schedule_structural();
        }
        if applied.refresh_display {
            // The display set changed: send frames (from the last complete
            // generation) or clears for the toggled nodes.
            self.core.refresh_display(inner, None);
        }
        Ok(())
    }

    /// Write the text (when it changed) and the sidecar, each atomically,
    /// and only then move `text_hash`. Text first — it is the source of
    /// truth — so a sidecar failure is the one half-way case, and it is
    /// undone here: the `before` text goes back over the one that landed.
    /// Should even that fail, the error says so; memory (rolled back by
    /// the caller) then differs from disk and the watcher's next pass
    /// reloads the disk as an external change — loud, never silent.
    fn persist(
        &self,
        inner: &mut Inner,
        text: &str,
        text_changed: bool,
        before: &StateSnapshot,
    ) -> Result<(), IntentError> {
        let pipeline = &self.core.config.pipeline;
        if text_changed {
            write_atomic(pipeline, text.as_bytes()).map_err(|e| {
                IntentError::Persist(format!("writing {}: {e}", display_path(pipeline)))
            })?;
        }
        if let Err(error) = inner.sidecar.save(&Sidecar::path_for(pipeline)) {
            use std::fmt::Write as _;
            let mut message = error.to_string();
            if text_changed {
                match write_atomic(pipeline, before.text.as_bytes()) {
                    Ok(()) => {
                        let _ = write!(
                            message,
                            "; the text write before it was taken back ({} is as it was)",
                            display_path(pipeline)
                        );
                    }
                    Err(detail) => {
                        let _ = write!(
                            message,
                            "; restoring {} failed too ({detail}) — the file holds the refused \
                             edit until the project watcher reloads it",
                            display_path(pipeline)
                        );
                    }
                }
            }
            return Err(IntentError::Persist(message));
        }
        if text_changed {
            inner.text_hash = *blake3::hash(text.as_bytes()).as_bytes();
        }
        Ok(())
    }

    // -------------------------------------------------------- apply_text --

    /// The atomic whole-file edit (docs/13 §Undo/redo; the ledger's `batch`
    /// operation for agents): refuse a stale base, a disallowed path, or a
    /// file that does not parse; else write every file temp + rename, swap
    /// the in-memory state, record ONE op, broadcast ONE delta (a snapshot
    /// when scripts changed and the catalog reloaded). Callable without a
    /// client (HTTP: the agent acts for the user, lease or no lease).
    /// Returns `{ok, seq, text_hash, history}`.
    ///
    /// # Errors
    ///
    /// [`IntentError::StaleBase`], [`IntentError::PathNotAllowed`],
    /// [`IntentError::ParseError`], [`IntentError::Io`] (every earlier file
    /// restored), [`IntentError::Protocol`] (empty / duplicate file list).
    #[allow(clippy::too_many_lines)] // validate → write → swap → record → broadcast, in order
    pub fn apply_text(
        &self,
        request: &ApplyTextRequest,
        source: DeltaSource,
    ) -> Result<serde_json::Value, IntentError> {
        if request.files.is_empty() {
            return Err(IntentError::Protocol(
                "apply_text with no files — nothing to apply".to_owned(),
            ));
        }
        // Classify every path first: a refusal must come before any write.
        let mut targets: Vec<(EditTarget, PathBuf, &str)> = Vec::new();
        for file in &request.files {
            let (target, absolute) = self.classify_edit_path(&file.path)?;
            if targets.iter().any(|(_, existing, _)| *existing == absolute) {
                return Err(IntentError::Protocol(format!(
                    "`{}` appears twice in the file list",
                    file.path
                )));
            }
            targets.push((target, absolute, file.text.as_str()));
        }
        let new_text = targets
            .iter()
            .find(|(t, _, _)| *t == EditTarget::Pipeline)
            .map(|(_, _, text)| (*text).to_owned());
        let new_sidecar = targets
            .iter()
            .find(|(t, _, _)| *t == EditTarget::Sidecar)
            .map(|(_, _, text)| {
                Sidecar::parse(text).map_err(|message| IntentError::ParseError {
                    message: format!("the sidecar does not parse: {message}"),
                    diagnostics: Vec::new(),
                })
            })
            .transpose()?;
        let scripts_changed = targets
            .iter()
            .any(|(t, _, _)| matches!(t, EditTarget::Script(_)));
        let new_document = new_text.as_deref().map(Document::parse);
        if let Some(document) = &new_document {
            let diagnostics = document.parse_diagnostics();
            if !diagnostics.is_empty() {
                return Err(IntentError::ParseError {
                    message: format!(
                        "the pipeline text does not parse ({} problem{}): {}",
                        diagnostics.len(),
                        if diagnostics.len() == 1 { "" } else { "s" },
                        diagnostics
                            .iter()
                            .map(|d| format!("line {}: {}", d.span.line, d.message))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                    diagnostics,
                });
            }
        }

        let mut inner = self.core.lock_inner();
        let current_hash = hex(&inner.text_hash);
        if request.base_text_hash != current_hash {
            return Err(IntentError::StaleBase {
                current_text_hash: current_hash,
            });
        }
        let before = state_snapshot(&inner);

        // What is on disk now, for the rollback of a half-applied write.
        let mut before_files: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
        for (_, absolute, _) in &targets {
            let existing = match std::fs::read(absolute) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(IntentError::Io(format!(
                        "reading {} before replacing it: {error}",
                        display_path(absolute)
                    )));
                }
            };
            before_files.push((absolute.clone(), existing));
        }
        // Write every file temp + rename, in order; a failure restores the
        // ones already replaced — the disk never keeps a partial edit.
        for (index, (target, absolute, text)) in targets.iter().enumerate() {
            let written = match (target, absolute.parent()) {
                // The first script of a project: `scripts/` may not exist.
                (EditTarget::Script(_), Some(parent)) => std::fs::create_dir_all(parent)
                    .and_then(|()| write_atomic(absolute, text.as_bytes())),
                _ => write_atomic(absolute, text.as_bytes()),
            };
            if let Err(error) = written {
                use std::fmt::Write as _;
                let mut message = format!("writing {}: {error}", display_path(absolute));
                let restored = restore_files(&before_files[..index]);
                if let Err(detail) = restored {
                    let _ = write!(
                        message,
                        "; restoring the earlier files failed too: {detail}"
                    );
                } else if index > 0 {
                    let _ = write!(
                        message,
                        "; the {index} file(s) written before it were restored"
                    );
                }
                return Err(IntentError::Io(message));
            }
        }

        // Swap the in-memory state. Scripts changed → the catalog reloads
        // (Python discovery) — everything fallible (the fingerprint of the
        // written scripts, the load) runs BEFORE memory moves, so a
        // failure has only the files to put back: a script that fails to
        // describe is a parse failure of the submitted edit, an unreadable
        // scripts dir an I/O one; the state stays as it was.
        if scripts_changed {
            let text_for_load = new_text
                .clone()
                .unwrap_or_else(|| inner.loaded.document.emit());
            let reloaded = scripts_fingerprint(&self.core.config.pipeline)
                .map_err(|e| IntentError::Io(e.to_string()))
                .and_then(|fresh| {
                    compile::load(
                        &self.core.config.pipeline,
                        &text_for_load,
                        &self.core.scripts,
                    )
                    .map(|loaded| (fresh, loaded))
                    .map_err(|error| IntentError::ParseError {
                        message: format!("a script does not load: {error}"),
                        diagnostics: Vec::new(),
                    })
                });
            match reloaded {
                Ok((fresh, loaded)) => {
                    inner.loaded = loaded;
                    inner.scripts_fingerprint = fresh;
                }
                Err(error) => {
                    use std::fmt::Write as _;
                    let mut message = error.to_string();
                    if let Err(detail) = restore_files(&before_files) {
                        let _ = write!(message, "; restoring the files failed too: {detail}");
                    }
                    return Err(match error {
                        IntentError::ParseError { diagnostics, .. } => IntentError::ParseError {
                            message,
                            diagnostics,
                        },
                        _ => IntentError::Io(message),
                    });
                }
            }
        } else if let Some(text) = &new_text {
            inner.loaded.reload_text(text);
        }
        let mut dirty = Vec::new();
        if let Some(text) = &new_text {
            let old = Document::parse(&before.text);
            dirty = changed_bindings(&old, &inner.loaded.document);
            inner.text_hash = *blake3::hash(text.as_bytes()).as_bytes();
        }
        if let Some(sidecar) = new_sidecar {
            inner.sidecar = sidecar;
        }
        self.core.rebuild(&mut inner);
        inner.seq += 1;
        let after = state_snapshot(&inner);
        if after != before {
            // An apply that left text and sidecar as they were (a
            // scripts-only change, a re-send of the current text) is not
            // an undo step — a snapshot op would restore nothing.
            let at = self.core.now_ms();
            inner.oplog.push(
                request.label.clone(),
                request.actor.clone(),
                before,
                after,
                at,
            );
        }
        if scripts_changed {
            // The catalog changed under the clients: one hydration path.
            let snapshot =
                self.core
                    .snapshot_locked(&inner, false, &format!("apply_text: {}", request.label));
            for client in inner.clients.values() {
                let _ = client.tx.send(Outgoing::Text(snapshot.clone()));
            }
        } else {
            let message = ServerMessage::Delta {
                source,
                graph: inner.graph.clone(),
                text: inner.loaded.document.emit(),
                dirty,
                history: inner.oplog.view(),
            };
            broadcast(&inner, &message);
        }
        if new_text.is_some() || scripts_changed {
            self.core.seed_statuses_locked(&inner);
            self.schedule_structural();
        } else {
            self.core.refresh_display(&mut inner, None);
        }
        Ok(serde_json::json!({
            "ok": true,
            "seq": inner.seq,
            "text_hash": hex(&inner.text_hash),
            "history": inner.oplog.view(),
        }))
    }

    /// Which file of the project an `apply_text` path names — this
    /// pipeline, its sidecar, or a script next to it — and its absolute
    /// path. Anything else is refused: plain project-relative paths only
    /// (no absolute/rooted/drive forms, no `.`/`..`, no backslashes).
    fn classify_edit_path(&self, path: &str) -> Result<(EditTarget, PathBuf), IntentError> {
        use std::path::Component;
        let refuse = |why: &str| IntentError::PathNotAllowed(format!("`{path}`: {why}"));
        let candidate = Path::new(path);
        if path.is_empty()
            || candidate.is_absolute()
            || candidate.has_root()
            || path.contains('\\')
            || path.contains(':')
            || !candidate
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(refuse(
                "not a plain project-relative path (no absolute, rooted or drive forms, no \
                 `.`/`..` segments, forward slashes only)",
            ));
        }
        let pipeline = self.core.relative.as_str();
        let sidecar = format!("{pipeline}.layout.json");
        let scripts_dir = match pipeline.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/scripts"),
            None => "scripts".to_owned(),
        };
        let target = if path == pipeline {
            EditTarget::Pipeline
        } else if path == sidecar {
            EditTarget::Sidecar
        } else if let Some((dir, name)) = path.rsplit_once('/')
            && dir == scripts_dir
            // Exactly what discovery picks up (`scripts.rs`: `ext == "py"`).
            && Path::new(name).extension().is_some_and(|ext| ext == "py")
            && name.len() > 3
        {
            EditTarget::Script(name.to_owned())
        } else {
            return Err(refuse(&format!(
                "an apply_text may replace this pipeline (`{pipeline}`), its sidecar \
                 (`{sidecar}`), or a script beside it (`{scripts_dir}/<name>.py`) — nothing else"
            )));
        };
        Ok((target, self.core.config.project_dir.join(path)))
    }

    /// Reload from disk after an external change (git checkout, editor):
    /// re-read text (+ scripts when `rescan_scripts`), sidecar, relower,
    /// clear the op log (the reload barrier, docs/13), broadcast a barrier
    /// snapshot, resolve. The session's own writes echo back through the
    /// watcher and are recognised because after them disk == memory —
    /// text by hash, sidecar by equality, scripts by the loaded
    /// fingerprint — and return `Ok(false)`. That equality is the WHOLE
    /// echo guard: anything on disk that differs from memory reloads,
    /// including a text this session once wrote and an external edit
    /// brought back.
    ///
    /// # Errors
    ///
    /// [`SessionError`] on unreadable files / script failures — the
    /// previous state stays live and the error is reported.
    pub fn reload_from_disk(
        &self,
        reason: &str,
        rescan_scripts: bool,
    ) -> Result<bool, SessionError> {
        let hold = self.hold_writes();
        self.reload_from_disk_held(hold, reason, rescan_scripts)
    }

    /// Exclude the session's own writes while a caller changes the files
    /// on disk (git revert: `checkout HEAD -- <paths>`), then hand the
    /// hold to [`Self::reload_from_disk_held`]. While the hold lives, no
    /// intent, `apply_text`, undo/redo, or watcher reload can touch the
    /// document — a slider drag that arrives during a revert applies to
    /// the REVERTED text afterwards instead of overwriting the restored
    /// file between the checkout and the reload (which would have made
    /// the reload a no-op and the revert silently lost). Keep it short:
    /// the whole session waits on it.
    pub fn hold_writes(&self) -> WriteHold<'_> {
        WriteHold(self.core.lock_inner())
    }

    /// [`Self::reload_from_disk`] under a hold taken before the files were
    /// changed: the reload sees exactly the caller's bytes, and the hold
    /// ends with the reload. Returns `Ok(false)` when disk equals memory.
    ///
    /// # Errors
    ///
    /// [`SessionError`] as for [`Self::reload_from_disk`].
    pub fn reload_from_disk_held(
        &self,
        hold: WriteHold<'_>,
        reason: &str,
        rescan_scripts: bool,
    ) -> Result<bool, SessionError> {
        {
            // Read UNDER the lock: a read racing a commit (two quick writes,
            // the watcher's read between them) would otherwise compare a
            // stale text against the newer hash and reload the old text
            // over the new one.
            let WriteHold(mut inner) = hold;
            let text = std::fs::read_to_string(&self.core.config.pipeline).map_err(|source| {
                SessionError::Read {
                    path: self.core.config.pipeline.clone(),
                    source,
                }
            })?;
            let hash = *blake3::hash(text.as_bytes()).as_bytes();
            let sidecar = Sidecar::load(&Sidecar::path_for(&self.core.config.pipeline))?;
            let fingerprint = rescan_scripts
                .then(|| scripts_fingerprint(&self.core.config.pipeline))
                .transpose()?;
            let text_same = inner.text_hash == hash;
            let sidecar_same = inner.sidecar == sidecar;
            let scripts_same = fingerprint
                .as_ref()
                .is_none_or(|fresh| *fresh == inner.scripts_fingerprint);
            if text_same && sidecar_same && scripts_same {
                // Our own write echoing back through the watcher (or a
                // touch that changed nothing).
                return Ok(false);
            }
            if rescan_scripts {
                let loaded = compile::load(&self.core.config.pipeline, &text, &self.core.scripts)?;
                inner.loaded = loaded;
                if let Some(fresh) = fingerprint {
                    inner.scripts_fingerprint = fresh;
                }
            } else if !text_same {
                inner.loaded.reload_text(&text);
            }
            inner.text_hash = hash;
            inner.sidecar = sidecar;
            // The barrier: an external change invalidates every snapshot
            // in the log — the stack is cleared, and says so when asked.
            inner.oplog.clear_by_barrier();
            self.core.rebuild(&mut inner);
            inner.seq += 1;
            // Barrier snapshot to everyone — UNDER the same lock, so it
            // describes the reloaded state and precedes on the wire any
            // edit that was waiting on the hold (otherwise that edit's
            // delta could go out first and the "barrier" would carry the
            // post-edit text).
            let snapshot = self.core.snapshot_locked(&inner, true, reason);
            for client in inner.clients.values() {
                let _ = client.tx.send(Outgoing::Text(snapshot.clone()));
            }
            self.core.seed_statuses_locked(&inner);
        }
        self.schedule_structural();
        Ok(true)
    }

    fn schedule_structural(&self) {
        let mut deadline = self
            .core
            .debounce
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *deadline = Some(Instant::now() + STRUCTURAL_DEBOUNCE);
        drop(deadline);
        self.core.debounce_wake.notify_all();
    }

    /// Submit the current document's solve immediately (initial load).
    pub fn submit_structural_now(&self) {
        self.core.submit_structural();
    }

    /// Block until the solve loop is idle (tests, `/debug/state?wait=1`).
    pub fn wait_idle(&self) {
        // Quiet means: no armed debounce AND no queued/in-flight generation,
        // observed in one pass (an edit arms the debounce; the debounce
        // thread submits before disarming; so looping until both are clear
        // covers every interleaving).
        loop {
            let pending = self
                .core
                .debounce
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some();
            if pending {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            self.solve.wait_idle();
            let rearmed = self
                .core
                .debounce
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some();
            if !rearmed && !self.solve.is_busy() {
                break;
            }
        }
        self.core.flush_status(true);
    }

    /// Cancel (Esc): the running generation stops and a queued edit's
    /// solve is dropped; the summary says so and the next edit resubmits.
    pub fn cancel(&self) {
        let dropped_pending = self.solve.cancel();
        if dropped_pending {
            let inner = self.core.lock_inner();
            broadcast(
                &inner,
                &ServerMessage::Notice {
                    level: "info".to_owned(),
                    message: "cancelled — the latest edit's solve was dropped too; any edit or                               slider move resumes solving"
                        .to_owned(),
                },
            );
        }
    }

    // -------------------------------------------------- effectful runs --

    /// Run an effectful node explicitly (`POST /api/run/{node}`, doc 10
    /// §7). Solves its cone on the shared scheduler with its own token
    /// (a slider drag never cancels an export). Blocking.
    ///
    /// # Errors
    ///
    /// The refusal text: unknown node, not effectful, diagnostics in the
    /// cone (JSON), red nodes.
    #[allow(clippy::too_many_lines)] // gate → lower → solve → statuses → timing → report, in order
    pub fn run_effectful(&self, node: &str) -> Result<serde_json::Value, String> {
        let (lowered, generation) = {
            let inner = self.core.lock_inner();
            let effectful =
                compile::effectful_bindings(&inner.loaded.document, &inner.loaded.specs);
            if inner.loaded.document.find_binding(node).is_none() {
                return Err(format!("no binding named `{node}`"));
            }
            if !effectful.contains(node) {
                return Err(format!(
                    "`{node}` is not effectful — it solves live; explicit runs are for exporters"
                ));
            }
            let targets = vec![node.to_owned()];
            let gate = compile::gate(
                &inner.loaded.document,
                &inner.loaded.resolution.diagnostics,
                &targets,
            );
            if !gate.blocking.is_empty() {
                return Err(format!(
                    "{} diagnostic(s) in `{node}`'s cone: {}",
                    gate.blocking.len(),
                    serde_json::to_string(&gate.blocking).unwrap_or_default()
                ));
            }
            let lowered = lower(
                &inner.loaded.document,
                &inner.loaded.resolution,
                &inner.loaded.specs,
                &self.core.config.project,
                &targets,
                &inner.loaded.scripts,
            )
            .map_err(|e| e.to_string())?;
            (Arc::new(lowered), self.solve.next_generation())
        };
        let Some(id) = lowered.graph.find(node) else {
            return Err(format!("`{node}` lowered to no node — bug"));
        };
        // Explicit runs never touch the live solve bar (summary/ETA belong
        // to the loop's generation, which may be in flight): only per-node
        // statuses move.
        let observer = ForwardObserver {
            core: Arc::clone(&self.core),
            generation,
        };
        {
            let mut status = self.core.lock_status();
            set_status(
                &mut status,
                node,
                NodeStatus::new(NodeState::Queued, generation),
            );
        }
        let started = Instant::now();
        let report = self
            .core
            .scheduler
            .solve(
                &lowered.graph,
                &[id],
                generation,
                &CancelToken::new(),
                &observer,
            )
            .map_err(|e| e.to_string())?;
        self.core
            .finish_statuses(generation, &lowered, &report, false);
        self.core.record_timing(GenerationTiming {
            generation,
            kind: "explicit",
            started_ms: (started - self.core.epoch).as_secs_f64() * 1000.0,
            queued_ms: 0.0,
            elapsed_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            cancelled: report.cancelled,
            cancel_to_idle_ms: None,
            computed: report
                .outcomes
                .iter()
                .filter(|o| matches!(o, NodeOutcome::Computed { .. }))
                .count(),
            cached: report
                .outcomes
                .iter()
                .filter(|o| matches!(o, NodeOutcome::CacheHit { .. }))
                .count(),
            frame_bytes: 0,
        });
        self.core.flush_status(true);
        let failures = report.failures();
        let ok = failures.is_empty() && !report.cancelled;
        let message = if ok {
            format!("`{node}` ran")
        } else {
            failures
                .iter()
                .map(|f| format!("red: `{}` — {}", f.node, f.message))
                .collect::<Vec<_>>()
                .join("; ")
        };
        {
            let inner = self.core.lock_inner();
            broadcast(
                &inner,
                &ServerMessage::RunFinished {
                    node: node.to_owned(),
                    ok,
                    message: message.clone(),
                },
            );
        }
        if ok {
            Ok(
                serde_json::json!({ "ok": true, "node": node, "generation": generation, "message": message }),
            )
        } else {
            Err(message)
        }
    }

    // ------------------------------------------------------ screenshots --

    /// Ask a connected client (the writer, else any) to render the
    /// viewport; resolves with PNG bytes. Returns `None` when no client is
    /// connected (the caller reports 503, loudly).
    #[must_use]
    pub fn request_screenshot(
        &self,
        target: &str,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<Vec<u8>, String>>> {
        let mut inner = self.core.lock_inner();
        let chosen = inner
            .writer
            .filter(|w| inner.clients.contains_key(w))
            .or_else(|| inner.clients.keys().next().copied())?;
        inner.next_screenshot += 1;
        let id = inner.next_screenshot;
        let (tx, rx) = tokio::sync::oneshot::channel();
        inner.screenshots.insert(id, tx);
        let message = ServerMessage::ScreenshotRequest {
            id,
            target: target.to_owned(),
        };
        send_to(&inner, chosen, &message);
        Some(rx)
    }

    // -------------------------------------------------------------- api --

    /// The project-aware catalog (stdlib + this pipeline's script nodes).
    #[must_use]
    pub fn catalog_value(&self) -> serde_json::Value {
        let inner = self.core.lock_inner();
        crate::catalog::catalog_value(&inner.loaded.specs)
    }

    /// `GET /api/blob/{hash}`: the summary of a stored value.
    ///
    /// # Errors
    ///
    /// A message when the hash is malformed or not in the store.
    pub fn blob_summary(&self, hash_hex: &str) -> Result<serde_json::Value, String> {
        let bytes = (0..32)
            .map(|i| {
                hash_hex
                    .get(2 * i..2 * i + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect::<Option<Vec<u8>>>()
            .filter(|_| hash_hex.len() == 64)
            .ok_or_else(|| format!("`{hash_hex}` is not a 64-hex-char blake3 hash"))?;
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&bytes);
        let value = self
            .core
            .scheduler
            .store()
            .load_value(&ValueHash::from_bytes(raw))
            .map_err(|e| e.to_string())?;
        serde_json::to_value(display::summarize(&value)).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------ debug --

    /// The `/debug/state` document. `with_values` adds per-node output
    /// summaries (loads values — opt in).
    #[must_use]
    pub fn debug_state(&self, with_values: bool) -> serde_json::Value {
        let inner = self.core.lock_inner();
        let status = self.core.lock_status();
        let display: BTreeMap<String, serde_json::Value> = inner
            .display
            .iter()
            .filter_map(|(&(node_ref, output), displayed)| {
                let name = inner.refs.name_of(node_ref)?;
                let port = inner
                    .graph
                    .node(name)
                    .and_then(|n| n.outputs.get(output as usize))
                    .map_or_else(|| output.to_string(), |o| o.name.clone());
                Some((
                    format!("{name}.{port}"),
                    serde_json::json!({
                        "hash": displayed.hash.to_hex(),
                        "generation": displayed.generation,
                        "stats": displayed.stats,
                    }),
                ))
            })
            .collect();
        let values: Option<BTreeMap<String, serde_json::Value>> = with_values.then(|| {
            inner
                .graph
                .nodes
                .iter()
                .map(|node| {
                    let (generation, outputs) = self.core.node_values(&inner, &node.name);
                    (
                        node.name.clone(),
                        serde_json::json!({ "generation": generation, "outputs": outputs }),
                    )
                })
                .collect()
        });
        serde_json::json!({
            "protocol": crate::protocol::PROTOCOL_VERSION,
            "engine": format!("cicada {}", env!("CARGO_PKG_VERSION")),
            "project": display_path(&self.core.config.project_dir),
            "pipeline": self.core.relative,
            "cache_dir": self.core.scheduler.store().root().to_string_lossy(),
            "threads": self.core.threads,
            "seq": inner.seq,
            "text": inner.loaded.document.emit(),
            "text_hash": hex(&inner.text_hash),
            "history": inner.oplog.view(),
            "ops": inner.oplog.debug_ops(),
            "graph": inner.graph,
            "statuses": status.nodes,
            "summary": status.summary,
            "solve": {
                "busy": self.solve.is_busy(),
                "last_complete_generation": inner.last_complete.as_ref().map(|k| k.generation),
            },
            "display": display,
            "lease": lease_view(&inner),
            "values": values,
            "timings": *self.core.timings.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.core.shutdown.store(true, Ordering::SeqCst);
        self.core.debounce_wake.notify_all();
        if let Some(handle) = self.debouncer.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.ticker.take() {
            let _ = handle.join();
        }
    }
}

// ------------------------------------------------------------- helpers --

/// Forwards an explicit run's events into the shared status board.
struct ForwardObserver {
    core: Arc<Core>,
    generation: u64,
}

impl Observer for ForwardObserver {
    fn on_event(&self, event: &Event<'_>) {
        self.core.on_event_impl(self.generation, event, false);
    }
}

fn broadcast(inner: &Inner, message: &ServerMessage) {
    let text = encode(inner.seq, message);
    for client in inner.clients.values() {
        let _ = client.tx.send(Outgoing::Text(text.clone()));
    }
}

fn broadcast_binary(inner: &Inner, bytes: &Bytes) {
    for client in inner.clients.values() {
        let _ = client.tx.send(Outgoing::Binary(bytes.clone()));
    }
}

fn send_to(inner: &Inner, client: u32, message: &ServerMessage) {
    if let Some(c) = inner.clients.get(&client) {
        let _ = c.tx.send(Outgoing::Text(encode(inner.seq, message)));
    }
}

fn lease_view(inner: &Inner) -> LeaseView {
    LeaseView {
        writer: inner.writer,
        clients: inner.clients.iter().map(|(&id, c)| (id, c.role)).collect(),
    }
}

fn broadcast_lease(inner: &Inner) {
    let lease = lease_view(inner);
    for client in inner.clients.values() {
        let _ = client.tx.send(Outgoing::Text(encode(
            inner.seq,
            &ServerMessage::Lease {
                lease: lease.clone(),
                role: client.role,
            },
        )));
    }
}

/// The reference text for a wire source: a bare name for value bindings
/// (single-output calls, literals, expressions), `name.port` for a port
/// of a multi-output node.
fn reference_text(inner: &Inner, from: &WireEnd) -> Result<String, IntentError> {
    if inner.loaded.document.find_binding(&from.node).is_none() {
        return Err(IntentError::Unknown(format!(
            "no node named `{}`",
            from.node
        )));
    }
    match inner.loaded.resolution.bindings.get(&from.node) {
        Some(BindingType::Node { .. }) => Ok(format!("{}.{}", from.node, from.port)),
        // Value / poisoned / unknown: bare reference; the checker decides
        // what it means after the edit.
        _ => Ok(from.node.clone()),
    }
}

/// The target call's port names in spec order (for kwarg insertion).
fn spec_order(inner: &Inner, node: &str) -> Option<Vec<&'static str>> {
    let line = inner.loaded.document.find_binding(node)?;
    let cicada_lang::Line::Statement { statement, .. } = &inner.loaded.document.lines()[line]
    else {
        return None;
    };
    let Rhs::Call(call) = &statement.rhs else {
        return None;
    };
    let spec = inner
        .loaded
        .specs
        .iter()
        .find(|spec| spec.name == call.func.name)?;
    Some(spec.inputs.iter().map(|port| port.name).collect())
}

fn apply_param(
    document: &mut Document,
    node: &str,
    port: Option<&str>,
    value: &str,
) -> Result<(), IntentError> {
    // A param edit writes ONE literal (docs/10: slider drag = one numeric
    // token). Anything else — a reference, an `each()`, a stray
    // expression — is refused before it touches the text.
    let trial = Document::parse(&format!("# cicada 1\n_probe = f(x={value})\n"));
    let is_literal = trial.statements().next().is_some_and(|(_, statement, _)| {
        matches!(&statement.rhs, Rhs::Call(call) if call
            .kwargs
            .first()
            .is_some_and(|k| matches!(k.value, cicada_lang::ast::ValueExpr::Literal(_))))
    });
    if !is_literal {
        return Err(IntentError::Refused(format!(
            "`{value}` is not a literal — param edits write one literal token (numbers, True/False, \"text\")"
        )));
    }
    match port {
        Some(port) => writer::set_param(document, node, port, value)?,
        None => writer::set_literal(document, node, value)?,
    }
    Ok(())
}

/// blake3 bytes → lowercase hex (the protocol's hash spelling).
fn hex(hash: &[u8; 32]) -> String {
    blake3::Hash::from_bytes(*hash).to_hex().to_string()
}

/// Which project file an `apply_text` path names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EditTarget {
    /// This pipeline's `.cic`.
    Pipeline,
    /// Its `.cic.layout.json`.
    Sidecar,
    /// `scripts/<name>.py` beside it.
    Script(String),
}

/// Run a write arm's body with a [`RollbackPoint`] around it: whatever
/// the body mutated in memory goes back under this same lock hold when it
/// fails. The body's commit takes care of the disk.
fn write_or_roll_back(
    inner: &mut Inner,
    body: impl FnOnce(&mut Inner) -> Result<(), IntentError>,
) -> Result<(), IntentError> {
    let point = RollbackPoint::capture(inner);
    let result = body(inner);
    if result.is_err() {
        point.restore(inner);
    }
    result
}

/// The current state as an op-log snapshot.
fn state_snapshot(inner: &Inner) -> StateSnapshot {
    StateSnapshot {
        text: inner.loaded.document.emit(),
        sidecar: inner.sidecar.clone(),
    }
}

/// Put a snapshot back into memory (undo/redo) — the commit persists it.
/// Text unchanged → a sidecar-only commit (no solve; undo never recomputes
/// anyway — the restored state's node keys are warm).
fn restore_state(inner: &mut Inner, target: &StateSnapshot, label: String) -> Applied {
    let text_changed = inner.loaded.document.emit() != target.text;
    let sidecar_changed = inner.sidecar != target.sidecar;
    let mut dirty = Vec::new();
    if text_changed {
        let restored = Document::parse(&target.text);
        dirty = changed_bindings(&inner.loaded.document, &restored);
        inner.loaded.reload_text(&target.text);
    }
    inner.sidecar = target.sidecar.clone();
    Applied {
        label,
        dirty,
        text_changed,
        // A text change repaints through the structural solve; a
        // sidecar-only restore (preview toggles) re-emits from the last
        // complete generation.
        refresh_display: sidecar_changed && !text_changed,
    }
}

/// The live statements referencing `name` (kwarg refs, expression free
/// vars) — the nodes that go red when `name` is deleted or disabled.
fn dependents_of(document: &Document, name: &str) -> Vec<String> {
    document
        .statements()
        .filter(|(_, statement, _)| statement.references().iter().any(|r| r.name == name))
        .map(|(_, statement, _)| statement.name().to_owned())
        .collect()
}

/// The names bound on lines whose raw text differs between two documents
/// (either side) — the dirty set of a whole-text change.
fn changed_bindings(old: &Document, new: &Document) -> Vec<String> {
    fn index(document: &Document) -> BTreeMap<String, &str> {
        let mut map = BTreeMap::new();
        for line in document.lines() {
            match line {
                Line::Statement { statement, raw } => {
                    for target in &statement.targets {
                        map.insert(target.name.clone(), raw.as_str());
                    }
                }
                Line::Disabled {
                    raw,
                    name: Some(name),
                    ..
                }
                | Line::Broken {
                    raw,
                    node: Some(name),
                    ..
                } => {
                    map.insert(name.clone(), raw.as_str());
                }
                _ => {}
            }
        }
        map
    }
    let before = index(old);
    let after = index(new);
    let mut dirty: Vec<String> = Vec::new();
    for (name, raw) in &after {
        if before.get(name) != Some(raw) {
            dirty.push(name.clone());
        }
    }
    for name in before.keys() {
        if !after.contains_key(name) {
            dirty.push(name.clone());
        }
    }
    dirty
}

/// blake3 of every `scripts/*.py` beside `pipeline`, by file name (empty
/// when there is no scripts dir).
fn scripts_fingerprint(pipeline: &Path) -> Result<BTreeMap<String, [u8; 32]>, SessionError> {
    let dir = pipeline.parent().unwrap_or(Path::new(".")).join("scripts");
    let mut out = BTreeMap::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let entries = std::fs::read_dir(&dir).map_err(|source| SessionError::Read {
        path: dir.clone(),
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| SessionError::Read {
                path: dir.clone(),
                source,
            })?
            .path();
        if path.extension().is_none_or(|ext| ext != "py") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|source| SessionError::Read {
            path: path.clone(),
            source,
        })?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.insert(name, *blake3::hash(&bytes).as_bytes());
    }
    Ok(out)
}

/// Put files back as they were before a half-applied `apply_text`
/// (`None` = the file did not exist). Every file is attempted; the
/// failures are reported together.
fn restore_files(files: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), String> {
    let mut failed = Vec::new();
    for (path, before) in files {
        let result = match before {
            Some(bytes) => write_atomic(path, bytes),
            None => match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
        };
        if let Err(error) = result {
            failed.push(format!("{}: {error}", display_path(path)));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// The checker's verdict on wiring `text` into `node.port` of `document`
/// (evaluated on a scratch copy): `(verdict, reason)` with verdict one of
/// `ok` / `lift` / `blocked`. The single source of wire truth for both the
/// drag-time probe and the connect gesture itself.
fn wire_verdict(
    document: &Document,
    specs: &[&'static NodeSpec],
    catalog: &Catalog<'_>,
    base_cycles: usize,
    node: &str,
    port: &str,
    text: &str,
) -> (String, Option<String>) {
    let mut scratch = document.clone();
    let order = spec_order_doc(&scratch, specs, node);
    if let Err(error) = writer::set_kwarg(&mut scratch, node, port, text, order.as_deref()) {
        return ("blocked".to_owned(), Some(error.to_string()));
    }
    let resolution = resolve(&scratch, catalog);
    let line = scratch.find_binding(node);
    let span = line.and_then(|line| match &scratch.lines()[line] {
        cicada_lang::Line::Statement { statement, .. } => match &statement.rhs {
            Rhs::Call(call) => call
                .kwargs
                .iter()
                .find(|k| k.name.name == port)
                .map(|k| (line, k.value.span())),
            _ => None,
        },
        _ => None,
    });
    let cycles = resolution
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Cycle)
        .count();
    if cycles > base_cycles {
        return (
            "blocked".to_owned(),
            Some("would create a cycle".to_owned()),
        );
    }
    let Some((line, span)) = span else {
        return (
            "blocked".to_owned(),
            Some("kwarg vanished after the edit — bug".to_owned()),
        );
    };
    let hit = resolution.diagnostics.iter().find(|d| {
        d.span.line == line + 1 && d.span.col_start < span.end && d.span.col_end > span.start
    });
    match hit {
        None => ("ok".to_owned(), None),
        Some(d) if d.kind == DiagnosticKind::NeedsLift => {
            let levels = d
                .fix
                .as_ref()
                .map_or(1, |fix| if fix.label.contains('×') { 2 } else { 1 });
            if levels == 1 {
                ("lift".to_owned(), Some(d.message.clone()))
            } else {
                (
                    "blocked".to_owned(),
                    Some(format!(
                        "{} — nested each() (depth > 1) is not executable in the spike (v0.1)",
                        d.message
                    )),
                )
            }
        }
        Some(d) if d.kind == DiagnosticKind::NeedsAdapter => (
            "blocked".to_owned(),
            Some(format!(
                "{} (adapter chips arrive with `insert_between`, v0.1 — place the adapter node)",
                d.message
            )),
        ),
        Some(d) => ("blocked".to_owned(), Some(d.message.clone())),
    }
}

/// The connect gesture, gated by the checker: the port must exist and the
/// wire must be `ok` (or `lift` with the chip accepted) — the text is edited
/// only when the checker agrees (docs/09: the wrong wire never exists).
fn connect_checked(
    inner: &mut Inner,
    from: &WireEnd,
    node: &str,
    port: &str,
    lift: bool,
) -> Result<(), IntentError> {
    let text = reference_text(inner, from)?;
    let spec_ports = spec_order(inner, node);
    if let Some(ports) = &spec_ports
        && !ports.contains(&port)
    {
        return Err(IntentError::Refused(format!(
            "`{node}` has no port `{port}` (ports: {})",
            ports.join(", ")
        )));
    }
    let catalog = Catalog::new(&inner.loaded.specs);
    let base_cycles = inner
        .loaded
        .resolution
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Cycle)
        .count();
    let (verdict, reason) = wire_verdict(
        &inner.loaded.document,
        &inner.loaded.specs,
        &catalog,
        base_cycles,
        node,
        port,
        &text,
    );
    match verdict.as_str() {
        "ok" => {}
        "lift" if lift => {}
        "lift" => {
            return Err(IntentError::Refused(format!(
                "{} — accept the lift chip (each()) to connect",
                reason.unwrap_or_default()
            )));
        }
        _ => {
            return Err(IntentError::Refused(format!(
                "wire {}.{} → {node}.{port} is blocked: {}",
                from.node,
                from.port,
                reason.unwrap_or_else(|| "incompatible".to_owned())
            )));
        }
    }
    let order = spec_ports;
    writer::set_kwarg(
        &mut inner.loaded.document,
        node,
        port,
        &text,
        order.as_deref(),
    )?;
    if lift {
        writer::wrap_each(&mut inner.loaded.document, node, port)?;
    }
    Ok(())
}

/// Every non-effectful node — the live solve pulls everything solvable
/// (exporters never auto-run, doc 10 §7).
fn all_targets(lowered: &Lowered) -> Vec<NodeId> {
    lowered
        .graph
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, node)| !node.effectful)
        .map(|(index, _)| NodeId(index))
        .collect()
}

/// Wire-drag verdicts, computed by asking the checker: for every input
/// port on the canvas (and every catalog node's ports), apply the wire to
/// a scratch copy and read the diagnostic on that kwarg — no second copy
/// of the type lattice anywhere.
#[allow(clippy::too_many_lines)] // the verdict closure + two sweeps (canvas ports, catalog)
fn probe_wire(
    inner: &Inner,
    from: &WireEnd,
) -> Result<(Vec<ProbeVerdict>, Vec<ProbeCatalogEntry>), IntentError> {
    let text = reference_text(inner, from)?;
    let catalog = Catalog::new(&inner.loaded.specs);
    let base_cycles = inner
        .loaded
        .resolution
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Cycle)
        .count();
    let verdict_for = |document: &Document, node: &str, port: &str| -> (String, Option<String>) {
        wire_verdict(
            document,
            &inner.loaded.specs,
            &catalog,
            base_cycles,
            node,
            port,
            &text,
        )
    };

    let mut targets = Vec::new();
    for node in &inner.graph.nodes {
        if node.name == from.node {
            continue;
        }
        if node.kind == viewmodel::NodeKind::Expression {
            // Expression inputs are the expression's free variables — a
            // wire there would mean rewriting the formula, which is not a
            // wire gesture. Say so instead of staying silent.
            for input in &node.inputs {
                targets.push(ProbeVerdict {
                    node: node.name.clone(),
                    port: input.name.clone(),
                    verdict: "blocked".to_owned(),
                    reason: Some(format!(
                        "`{}` is a free variable of the expression — edit the formula text \
                         (or rename the source binding) rather than wiring",
                        input.name
                    )),
                });
            }
            continue;
        }
        if node.func.is_none() || node.kind != viewmodel::NodeKind::Call {
            continue;
        }
        for input in &node.inputs {
            if input.unknown {
                continue;
            }
            let (verdict, reason) = verdict_for(&inner.loaded.document, &node.name, &input.name);
            targets.push(ProbeVerdict {
                node: node.name.clone(),
                port: input.name.clone(),
                verdict,
                reason,
            });
        }
    }
    let mut catalog_entries = Vec::new();
    for spec in &inner.loaded.specs {
        let mut scratch = inner.loaded.document.clone();
        let Ok(name) = writer::place(&mut scratch, spec.name, &[]) else {
            continue;
        };
        let mut ports = Vec::new();
        for port in spec.inputs {
            let (verdict, _) = verdict_for(&scratch, &name, port.name);
            if verdict != "blocked" {
                ports.push((port.name.to_owned(), verdict));
            }
        }
        if !ports.is_empty() {
            catalog_entries.push(ProbeCatalogEntry {
                func: spec.name.to_owned(),
                ports,
            });
        }
    }
    Ok((targets, catalog_entries))
}

fn spec_order_doc(
    document: &Document,
    specs: &[&'static NodeSpec],
    node: &str,
) -> Option<Vec<&'static str>> {
    let line = document.find_binding(node)?;
    let cicada_lang::Line::Statement { statement, .. } = &document.lines()[line] else {
        return None;
    };
    let Rhs::Call(call) = &statement.rhs else {
        return None;
    };
    let spec = specs.iter().find(|spec| spec.name == call.func.name)?;
    Some(spec.inputs.iter().map(|port| port.name).collect())
}

fn debounce_loop(core: &Core) {
    loop {
        let deadline = {
            let guard = core
                .debounce
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let guard = core
                .debounce_wake
                .wait_while(guard, |deadline| {
                    deadline.is_none() && !core.shutdown.load(Ordering::SeqCst)
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if core.shutdown.load(Ordering::SeqCst) {
                return;
            }
            let Some(deadline) = *guard else { continue };
            deadline
        };
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
            // A newer edit may have pushed the deadline; loop re-checks.
            let current = *core
                .debounce
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if current != Some(deadline) {
                continue;
            }
        }
        // Submit FIRST, then clear the deadline: a `wait_idle` caller always
        // sees either the armed debounce or the queued/in-flight job — never
        // a gap in which the edit is nowhere (probe friction: stale oracle).
        core.submit_structural();
        *core
            .debounce
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

impl Core {
    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, StatusBoard> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn take_notices(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .notices
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Op-log timestamp: monotonic milliseconds from the op clock.
    fn now_ms(&self) -> u64 {
        self.op_clock.now_nanos() / 1_000_000
    }

    /// The `snapshot` message for the current state (`inner` held; the
    /// status board is locked inside — the inner → status order every
    /// holder of both follows).
    fn snapshot_locked(&self, inner: &Inner, barrier: bool, reason: &str) -> String {
        let status = self.lock_status();
        encode(
            inner.seq,
            &ServerMessage::Snapshot {
                graph: inner.graph.clone(),
                text: inner.loaded.document.emit(),
                statuses: status.nodes.clone(),
                summary: status.summary.clone(),
                lease: lease_view(inner),
                barrier,
                reason: reason.to_owned(),
                history: inner.oplog.view(),
            },
        )
    }

    /// Relower + rebuild the view-model from the current document,
    /// resolution, and sidecar; clear frames of nodes that vanished.
    fn rebuild(&self, inner: &mut Inner) {
        let lowered = lower_partial(
            &inner.loaded.document,
            &inner.loaded.resolution,
            &inner.loaded.specs,
            &self.config.project,
            &inner.loaded.scripts,
        );
        match lowered {
            Ok(lowered) => inner.lowered = Arc::new(lowered),
            Err(error) => {
                // Graph assembly failing is a bug; keep the previous graph
                // live and shout.
                self.notices
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(format!("lowering failed: {error}"));
            }
        }
        let Inner {
            loaded,
            sidecar,
            lowered,
            graph,
            refs,
            ..
        } = inner;
        *graph = viewmodel::build(
            &loaded.document,
            &loaded.resolution,
            &loaded.specs,
            lowered,
            sidecar,
            refs,
        );
        // Frames for nodes no longer on the canvas: clear.
        let live: HashSet<u32> = inner.graph.nodes.iter().map(|n| n.node_ref).collect();
        let gone: Vec<(u32, u32)> = inner
            .display
            .keys()
            .filter(|(node, _)| !live.contains(node))
            .copied()
            .collect();
        for key in gone {
            let displayed = inner.display.remove(&key);
            let generation = displayed.map_or(0, |d| d.generation);
            broadcast_binary(
                inner,
                &Bytes::from(display::clear_frame(generation, key.0, key.1)),
            );
        }
    }

    /// Submit a structural generation over the current lowered graph.
    fn submit_structural(&self) {
        let job = {
            let inner = self.lock_inner();
            Job {
                lowered: Arc::clone(&inner.lowered),
                targets: all_targets(&inner.lowered),
                kind: JobKind::Structural,
                submitted: Instant::now(),
            }
        };
        let solve = self
            .solve
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        if let Some(solve) = solve {
            solve.submit(job);
        }
    }

    /// Statuses for excluded nodes (red/blocked from the checker) —
    /// visible immediately, before any generation runs.
    fn seed_statuses(&self) {
        let inner = self.lock_inner();
        self.seed_statuses_locked(&inner);
    }

    fn seed_statuses_locked(&self, inner: &Inner) {
        let mut status = self.lock_status();
        let live: HashSet<&str> = inner.graph.nodes.iter().map(|n| n.name.as_str()).collect();
        let vanished: Vec<String> = status
            .nodes
            .keys()
            .filter(|name| !live.contains(name.as_str()))
            .cloned()
            .collect();
        for name in vanished {
            status.nodes.remove(&name);
        }
        for node in &inner.graph.nodes {
            if let Some(excluded) = &node.excluded {
                let state = if excluded.status == "blocked" {
                    NodeState::Blocked
                } else {
                    NodeState::Red
                };
                let mut entry = NodeStatus::new(state, status.summary.generation);
                entry.message = Some(excluded.reason.clone());
                set_status(&mut status, &node.name, entry);
            } else if node.kind == viewmodel::NodeKind::Literal
                && !status.nodes.contains_key(&node.name)
            {
                // Literals never enter the graph — they are always "done".
                set_status(&mut status, &node.name, NodeStatus::new(NodeState::Done, 0));
            } else if node.effectful
                && status.nodes.get(&node.name).is_none_or(|existing| {
                    // Keep a real run's verdict (done/red from an explicit
                    // run); anything else — idle, or a stale blocked/red
                    // left by an old generation the exporter never joins —
                    // resets to the honest idle line.
                    !matches!(existing.state, NodeState::Done | NodeState::Running)
                        || existing.generation == 0
                })
            {
                // Exporters never auto-run (doc 10 §7): idle, and say why —
                // an honest status beats a node that never changes.
                let mut entry = NodeStatus::new(NodeState::Idle, 0);
                entry.message = Some(
                    "effectful — runs only on explicit action (Run in the inspector / POST /api/run)"
                        .to_owned(),
                );
                set_status(&mut status, &node.name, entry);
            } else if let Some(existing) = status.nodes.get(&node.name).cloned()
                && matches!(existing.state, NodeState::Red | NodeState::Blocked)
            {
                // Was red/blocked (excluded by the checker, or by an older
                // generation), no longer excluded — reset to idle until the
                // next generation speaks (it runs within the debounce).
                set_status(&mut status, &node.name, NodeStatus::new(NodeState::Idle, 0));
            }
        }
        status.dirty = true;
    }

    /// Frames for outputs whose hash changed since the last broadcast
    /// (or every displayed output when `only` is `None` after a preview
    /// toggle). Uses the last complete generation's report.
    fn refresh_display(&self, inner: &mut Inner, _only: Option<&str>) {
        let Some(kept) = inner.last_complete.as_ref() else {
            return;
        };
        let generation = kept.generation;
        let lowered = Arc::clone(&kept.lowered);
        let report = Arc::clone(&kept.report);
        self.emit_frames(inner, generation, &lowered, &report);
    }

    /// The core of display: for every previewed, displayable output in the
    /// graph, compare the generation's output hash to the last broadcast
    /// and send frames when it changed; send clears for outputs that
    /// stopped drawing.
    #[allow(clippy::too_many_lines)] // want-set, clears, sends: one pass in one place
    fn emit_frames(
        &self,
        inner: &mut Inner,
        generation: u64,
        lowered: &Lowered,
        report: &SolveReport,
    ) -> usize {
        let mut bytes_sent = 0_usize;
        let store = Arc::clone(self.scheduler.store());
        let tolerance = self.config.project.tol();
        // Which (ref, output) should draw now?
        let mut wanted: Vec<(u32, u32, ValueHash)> = Vec::new();
        for node in &inner.graph.nodes {
            if !node.preview {
                continue;
            }
            for (output_index, output) in node.outputs.iter().enumerate() {
                if !output.displayable {
                    continue;
                }
                let hash = match lowered.bindings.get(&node.name) {
                    Some(LoweredBinding::Value(value)) => value.hash(),
                    Some(LoweredBinding::Port { node: id, output }) => {
                        // Single-output binding: the port index is the
                        // binding's own; unpack targets map by name below.
                        let Some(hashes) = report.outcome(*id).output_hashes() else {
                            continue;
                        };
                        let index = if node.targets.len() > 1 {
                            // Multi-target unpack: this OUTPUT's own binding.
                            match node
                                .targets
                                .get(output_index)
                                .and_then(|t| lowered.bindings.get(t))
                            {
                                Some(LoweredBinding::Port { output, .. }) => *output,
                                _ => *output,
                            }
                        } else {
                            *output
                        };
                        let Some(hash) = hashes.get(index) else {
                            continue;
                        };
                        *hash
                    }
                    Some(LoweredBinding::Node { node: id }) => {
                        let Some(hashes) = report.outcome(*id).output_hashes() else {
                            continue;
                        };
                        let Some(hash) = hashes.get(output_index) else {
                            continue;
                        };
                        *hash
                    }
                    None => continue,
                };
                let output_u32 = u32::try_from(output_index).unwrap_or(u32::MAX);
                wanted.push((node.node_ref, output_u32, hash));
            }
        }
        // Clears: displayed before, not wanted now (preview off, red, gone).
        let wanted_keys: HashSet<(u32, u32)> = wanted.iter().map(|(n, o, _)| (*n, *o)).collect();
        let stale: Vec<(u32, u32)> = inner
            .display
            .keys()
            .filter(|key| !wanted_keys.contains(key))
            .copied()
            .collect();
        for key in stale {
            // Red/blocked/cancelled outputs keep their last frame (docs/12:
            // last coherent value) — only outputs whose node is still
            // previewed but produced nothing are cleared here; nodes that
            // stopped being displayable/previewed clear too.
            let still_previewed_but_failed = inner.graph.nodes.iter().any(|n| {
                n.node_ref == key.0
                    && n.preview
                    && n.outputs.get(key.1 as usize).is_some_and(|o| o.displayable)
            });
            if still_previewed_but_failed {
                continue;
            }
            inner.display.remove(&key);
            broadcast_binary(
                inner,
                &Bytes::from(display::clear_frame(generation, key.0, key.1)),
            );
        }
        for (node_ref, output, hash) in wanted {
            if inner
                .display
                .get(&(node_ref, output))
                .is_some_and(|d| d.hash == hash)
            {
                continue;
            }
            match store.load_value(&hash) {
                Ok(value) => {
                    let frames = display::frames_for_value(
                        &value,
                        generation,
                        node_ref,
                        output,
                        &mut inner.picks,
                        tolerance,
                    );
                    for frame in frames.frames {
                        bytes_sent += frame.len();
                        broadcast_binary(inner, &Bytes::from(frame));
                    }
                    inner.display.insert(
                        (node_ref, output),
                        Displayed {
                            hash,
                            generation,
                            stats: frames.stats,
                        },
                    );
                }
                Err(error) => {
                    broadcast(
                        inner,
                        &ServerMessage::Notice {
                            level: "warning".to_owned(),
                            message: format!("value {hash} not loadable for display: {error}"),
                        },
                    );
                }
            }
        }
        bytes_sent
    }

    /// Per-output value summaries for a node from the last complete
    /// generation.
    fn node_values(&self, inner: &Inner, node: &str) -> (u64, Vec<(String, Option<ValueSummary>)>) {
        let Some(view) = inner.graph.node(node) else {
            return (0, Vec::new());
        };
        let Some(kept) = inner.last_complete.as_ref() else {
            return (
                0,
                view.outputs
                    .iter()
                    .map(|o| (o.name.clone(), None))
                    .collect(),
            );
        };
        let store = self.scheduler.store();
        let mut outputs = Vec::new();
        for (index, output) in view.outputs.iter().enumerate() {
            let hash = match kept.lowered.bindings.get(&view.name) {
                Some(LoweredBinding::Value(value)) => Some(value.hash()),
                Some(LoweredBinding::Port {
                    node: id,
                    output: port,
                }) => {
                    let port = if view.targets.len() > 1 {
                        match view
                            .targets
                            .get(index)
                            .and_then(|t| kept.lowered.bindings.get(t))
                        {
                            Some(LoweredBinding::Port { output, .. }) => *output,
                            _ => *port,
                        }
                    } else {
                        *port
                    };
                    kept.report
                        .outcome(*id)
                        .output_hashes()
                        .and_then(|h| h.get(port).copied())
                }
                Some(LoweredBinding::Node { node: id }) => kept
                    .report
                    .outcome(*id)
                    .output_hashes()
                    .and_then(|h| h.get(index).copied()),
                None => None,
            };
            let summary =
                hash.and_then(|hash| store.load_value(&hash).ok().map(|v| display::summarize(&v)));
            outputs.push((output.name.clone(), summary));
        }
        (kept.generation, outputs)
    }

    /// Fill statuses from a finished report.
    fn finish_statuses(
        &self,
        generation: u64,
        lowered: &Lowered,
        report: &SolveReport,
        with_summary: bool,
    ) {
        let mut status = self.lock_status();
        let saved = (!with_summary).then(|| (status.summary.clone(), status.started));
        for (index, outcome) in report.outcomes.iter().enumerate() {
            let name = lowered.graph.node(NodeId(index)).name.clone();
            let entry = match outcome {
                NodeOutcome::Skipped => continue,
                NodeOutcome::CacheHit { .. } => NodeStatus::new(NodeState::Cached, generation),
                NodeOutcome::Computed {
                    elements, nanos, ..
                } => {
                    let mut s = NodeStatus::new(NodeState::Done, generation);
                    s.elements = Some(*elements);
                    s.nanos = Some(*nanos);
                    s
                }
                NodeOutcome::Failed(failure) => {
                    let mut s = NodeStatus::new(NodeState::Red, generation);
                    s.message = Some(failure.message.clone());
                    s.element_ids.clone_from(&failure.element_ids);
                    s
                }
                NodeOutcome::Blocked { upstream } => {
                    let mut s = NodeStatus::new(NodeState::Blocked, generation);
                    s.message = Some(format!("fed by red `{upstream}`"));
                    s
                }
                NodeOutcome::Cancelled => NodeStatus::new(NodeState::Cancelled, generation),
            };
            set_status(&mut status, &name, entry);
        }
        let mut computed = 0;
        let mut cached = 0;
        for outcome in &report.outcomes {
            match outcome {
                NodeOutcome::Computed { .. } => computed += 1,
                NodeOutcome::CacheHit { .. } => cached += 1,
                _ => {}
            }
        }
        // Red/blocked count EVERY node wearing the word — checker-excluded
        // ones (which never enter the solve) included — so the solve bar
        // and the canvas badges agree.
        let red = status
            .nodes
            .values()
            .filter(|s| s.state == NodeState::Red)
            .count();
        let blocked = status
            .nodes
            .values()
            .filter(|s| s.state == NodeState::Blocked)
            .count();
        let elapsed = status
            .started
            .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
        status.summary = SolveSummary {
            generation,
            running: false,
            cancelled: report.cancelled,
            computed,
            cached,
            pending: 0,
            red,
            blocked,
            elapsed_ms: elapsed,
            eta_ms: None,
            eta_rough: false,
        };
        status.started = None;
        if let Some((summary, started)) = saved {
            status.summary = summary;
            status.started = started;
        }
        status.dirty = true;
    }

    /// Record one generation's timing (bounded ring).
    fn record_timing(&self, timing: GenerationTiming) {
        let mut timings = self
            .timings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if timings.len() >= TIMINGS_KEPT {
            timings.pop_front();
        }
        timings.push_back(timing);
    }

    /// Flush changed statuses to every client (ticker: only when dirty;
    /// `force` = generation boundaries).
    fn flush_status(&self, force: bool) {
        let payload = {
            let mut status = self.lock_status();
            if !status.dirty && !force && !status.summary.running {
                return;
            }
            if status.summary.running
                && let Some(started) = status.started
            {
                status.summary.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let (eta, rough) = eta(&status, self.threads);
                status.summary.eta_ms = eta;
                status.summary.eta_rough = rough;
            }
            let changed = std::mem::take(&mut status.changed);
            status.dirty = false;
            if changed.is_empty() && !force && !status.summary.running {
                return;
            }
            ServerMessage::Status {
                generation: status.summary.generation,
                nodes: changed,
                summary: status.summary.clone(),
            }
        };
        let inner = self.lock_inner();
        broadcast(&inner, &payload);
    }
}

fn set_status(status: &mut StatusBoard, name: &str, entry: NodeStatus) {
    if status.nodes.get(name) == Some(&entry) {
        return;
    }
    status.nodes.insert(name.to_owned(), entry.clone());
    status.changed.insert(name.to_owned(), entry);
    status.dirty = true;
}

/// Cost-weighted ETA from persisted samples: Σ predicted nanos of pending
/// nodes ÷ threads. `rough` when any pending op has no samples yet.
fn eta(status: &StatusBoard, threads: usize) -> (Option<f64>, bool) {
    let mut total: u64 = 0;
    let mut rough = false;
    let mut any = false;
    for (name, node) in &status.nodes {
        if matches!(node.state, NodeState::Queued | NodeState::Running) {
            any = true;
            match status.predicted.get(name).copied().flatten() {
                Some(nanos) => total = total.saturating_add(nanos),
                None => rough = true,
            }
        }
    }
    if !any {
        return (None, false);
    }
    #[allow(clippy::cast_precision_loss)]
    let ms = total as f64 / 1_000_000.0 / threads.max(1) as f64;
    (Some(ms), rough)
}

impl SolveSink for Core {
    fn on_start(&self, generation: u64, job: &Job) {
        let mut status = self.lock_status();
        status.summary = SolveSummary {
            generation,
            running: true,
            ..SolveSummary::default()
        };
        let now = Instant::now();
        status.started = Some(now);
        status.queued = Some(now.saturating_duration_since(job.submitted));
        status.predicted.clear();
        let cone = job.lowered.graph.ancestors(&job.targets);
        let store = self.scheduler.store();
        let mut pending = 0;
        for (index, in_cone) in cone.iter().enumerate() {
            if !in_cone {
                continue;
            }
            let decl = job.lowered.graph.node(NodeId(index));
            pending += 1;
            // Cost-weighted prediction = per-element sample × the node's
            // LAST element count (a fan-out of 1,500 is not a fan-out of 1);
            // no sample or no known count → None → the ETA shows as rough.
            let last_elements = status
                .nodes
                .get(&decl.name)
                .and_then(|s| s.elements)
                .filter(|&n| n > 0);
            let predicted = store
                .stats(&decl.op)
                .and_then(|stats| stats.per_element_nanos())
                .and_then(|per| last_elements.map(|n| per.saturating_mul(n)));
            status.predicted.insert(decl.name.clone(), predicted);
            set_status(
                &mut status,
                &decl.name,
                NodeStatus::new(NodeState::Queued, generation),
            );
        }
        status.summary.pending = pending;
        status.dirty = true;
    }

    fn on_event(&self, generation: u64, event: &Event<'_>) {
        self.on_event_impl(generation, event, true);
    }

    fn on_complete(&self, generation: u64, job: &Job, report: Arc<SolveReport>) {
        self.on_complete_impl(generation, job, &report);
    }

    fn on_error(&self, generation: u64, _job: &Job, error: &SolveError) {
        self.on_error_impl(generation, error);
    }

    fn on_cancel_settled(&self, generation: u64, cancel_to_idle: Duration) {
        let mut timings = self
            .timings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The record was pushed by `on_complete`; an engine error leaves
        // none (nothing to annotate — the error notice already said so).
        if let Some(timing) = timings
            .iter_mut()
            .rev()
            .find(|t| t.generation == generation)
        {
            timing.cancel_to_idle_ms = Some(cancel_to_idle.as_secs_f64() * 1000.0);
        }
    }
}

impl Core {
    /// Apply one execution event to the status board. `touch_summary`
    /// is false for explicit runs — their counters must not disturb the
    /// live generation's solve bar.
    fn on_event_impl(&self, generation: u64, event: &Event<'_>, touch_summary: bool) {
        let mut status = self.lock_status();
        let saved = (!touch_summary).then(|| status.summary.clone());
        match event {
            Event::NodeStarted { node } => {
                set_status(
                    &mut status,
                    node,
                    NodeStatus::new(NodeState::Running, generation),
                );
            }
            Event::NodeCacheHit { node } => {
                set_status(
                    &mut status,
                    node,
                    NodeStatus::new(NodeState::Cached, generation),
                );
                status.summary.cached += 1;
                status.summary.pending = status.summary.pending.saturating_sub(1);
            }
            Event::ElementCacheHit { node, .. } => {
                let mut entry = status
                    .nodes
                    .get(*node)
                    .cloned()
                    .unwrap_or_else(|| NodeStatus::new(NodeState::Running, generation));
                entry.state = NodeState::Running;
                entry.elements_done = Some(entry.elements_done.unwrap_or(0) + 1);
                set_status(&mut status, node, entry);
            }
            Event::ChunkExecuted { node, len, .. } => {
                let mut entry = status
                    .nodes
                    .get(*node)
                    .cloned()
                    .unwrap_or_else(|| NodeStatus::new(NodeState::Running, generation));
                entry.state = NodeState::Running;
                entry.elements_done = Some(entry.elements_done.unwrap_or(0) + *len as u64);
                set_status(&mut status, node, entry);
            }
            Event::NodeComputed {
                node,
                elements,
                nanos,
            } => {
                let mut entry = NodeStatus::new(NodeState::Done, generation);
                entry.elements = Some(*elements);
                entry.nanos = Some(*nanos);
                set_status(&mut status, node, entry);
                status.summary.computed += 1;
                status.summary.pending = status.summary.pending.saturating_sub(1);
            }
            Event::NodeFailed { node } => {
                set_status(
                    &mut status,
                    node,
                    NodeStatus::new(NodeState::Red, generation),
                );
                status.summary.red += 1;
                status.summary.pending = status.summary.pending.saturating_sub(1);
            }
            Event::NodeBlocked { node, upstream } => {
                let mut entry = NodeStatus::new(NodeState::Blocked, generation);
                entry.message = Some(format!("fed by red `{upstream}`"));
                set_status(&mut status, node, entry);
                status.summary.blocked += 1;
                status.summary.pending = status.summary.pending.saturating_sub(1);
            }
            Event::NodeCancelled { node } => {
                set_status(
                    &mut status,
                    node,
                    NodeStatus::new(NodeState::Cancelled, generation),
                );
                status.summary.pending = status.summary.pending.saturating_sub(1);
            }
        }
        if let Some(summary) = saved {
            status.summary = summary;
        }
    }

    fn on_complete_impl(&self, generation: u64, job: &Job, report: &Arc<SolveReport>) {
        // Read the start/queue marks BEFORE the status board forgets them
        // (finishing the statuses clears `started`).
        let (started_ms, queued_ms) = {
            let status = self.lock_status();
            (
                status
                    .started
                    .map_or(0.0, |s| (s - self.epoch).as_secs_f64() * 1000.0),
                status.queued.map_or(0.0, |q| q.as_secs_f64() * 1000.0),
            )
        };
        self.finish_statuses(generation, &job.lowered, report, true);
        let began = Instant::now();
        let mut frame_bytes = 0;
        {
            let mut inner = self.lock_inner();
            let newer = inner
                .last_complete
                .as_ref()
                .is_none_or(|kept| generation > kept.generation);
            if newer && !report.cancelled {
                inner.last_complete = Some(Kept {
                    generation,
                    lowered: Arc::clone(&job.lowered),
                    report: Arc::clone(report),
                });
            }
            if newer && !report.cancelled {
                // Frames for what completed. A CANCELLED generation paints
                // nothing: its finished upstream outputs with the previous
                // generation's downstream ones would be an incoherent mix,
                // and doc 04 says a cancelled solve leaves the last
                // coherent frame — the previous generation's. (It also
                // cost the Esc latency: encoding ~100 MB of half-finished
                // wall cutters stood between cancel() and idle — stage-6
                // measurement.) The completed work is memoized; the next
                // generation paints it in milliseconds.
                frame_bytes = self.emit_frames(&mut inner, generation, &job.lowered, report);
            }
        }
        let elapsed = {
            let status = self.lock_status();
            status.summary.elapsed_ms
        } + began.elapsed().as_secs_f64() * 1000.0;
        self.record_timing(GenerationTiming {
            generation,
            kind: match job.kind {
                JobKind::Structural => "structural",
                JobKind::Preview => "preview",
            },
            started_ms,
            queued_ms,
            elapsed_ms: Some(elapsed),
            cancelled: report.cancelled,
            cancel_to_idle_ms: None,
            computed: report
                .outcomes
                .iter()
                .filter(|o| matches!(o, NodeOutcome::Computed { .. }))
                .count(),
            cached: report
                .outcomes
                .iter()
                .filter(|o| matches!(o, NodeOutcome::CacheHit { .. }))
                .count(),
            frame_bytes,
        });
        self.flush_status(true);
    }

    fn on_error_impl(&self, generation: u64, error: &SolveError) {
        {
            let mut status = self.lock_status();
            status.summary.running = false;
            status.summary.generation = generation;
            status.started = None;
            status.dirty = true;
        }
        let inner = self.lock_inner();
        broadcast(
            &inner,
            &ServerMessage::Notice {
                level: "error".to_owned(),
                message: format!("generation {generation} failed: {error}"),
            },
        );
        drop(inner);
        self.flush_status(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{Frame, FrameKind, decode};
    use tokio::sync::mpsc::unbounded_channel;

    fn project(source: &str) -> (tempfile::TempDir, SessionConfig) {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = dir.path().join("p.cic");
        std::fs::write(&pipeline, source).unwrap();
        let config = SessionConfig {
            project_dir: dir.path().to_owned(),
            pipeline,
            cache_dir: Some(dir.path().join("cache")),
            threads: 2,
            project: ProjectConfig::default(),
            op_clock: None,
        };
        (dir, config)
    }

    fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Outgoing>) -> Vec<Outgoing> {
        let mut out = Vec::new();
        while let Ok(message) = rx.try_recv() {
            out.push(message);
        }
        out
    }

    fn texts(messages: &[Outgoing]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .filter_map(|m| match m {
                Outgoing::Text(t) => serde_json::from_str(t).ok(),
                Outgoing::Binary(_) => None,
            })
            .collect()
    }

    fn frames(messages: &[Outgoing]) -> Vec<Frame> {
        messages
            .iter()
            .filter_map(|m| match m {
                Outgoing::Binary(b) => decode(b).ok(),
                Outgoing::Text(_) => None,
            })
            .collect()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one end-to-end story: open → frames → every gesture
    fn open_solve_and_stream_frames_then_edit_via_intents() {
        let (_dir, config) = project(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n",
        );
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, role) = session.connect(tx);
        assert_eq!(role, Role::Writer);
        session.restream_display(id);
        let got = drain(&mut rx);
        let all = frames(&got);
        let mesh_frames: Vec<&Frame> = all
            .iter()
            .filter(|f| f.header().kind == FrameKind::Mesh)
            .collect();
        assert_eq!(mesh_frames.len(), 1, "one displayed mesh output: block");
        let Frame::Batch { batch, header } = mesh_frames[0] else {
            panic!()
        };
        assert_eq!(batch.elements.len(), 1);
        assert_eq!(batch.indices.len(), 36, "a box: 12 triangles");
        let block_ref = header.node;
        let state = session.debug_state(false);
        assert_eq!(state["display"]["block.out"]["stats"]["triangles"], 12);
        let bounds_before = state["display"]["block.out"]["stats"]["bounds"].clone();
        assert_eq!(bounds_before[1][0], 2.0);
        assert_eq!(state["statuses"]["block"]["state"], "done");

        // Slider drag: preview streams, then the real set_param on release.
        session.handle(
            id,
            Some("p1".into()),
            ClientMessage::ParamPreview {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        let got = drain(&mut rx);
        let preview_frames = frames(&got);
        assert!(
            preview_frames
                .iter()
                .any(|f| f.header().kind == FrameKind::Mesh && f.header().node == block_ref),
            "the preview repaints the box"
        );
        assert_eq!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .matches("value=2.0")
                .count(),
            1,
            "previews never write the file"
        );
        session.handle(
            id,
            Some("s1".into()),
            ClientMessage::SetParam {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        let text = std::fs::read_to_string(&pipeline).unwrap();
        assert!(
            text.contains("size = slider(value=3.0, min=0.5, max=5.0)"),
            "{text}"
        );
        let got = drain(&mut rx);
        let msgs = texts(&got);
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert_eq!(delta["payload"]["source"]["intent_id"], "s1");
        assert_eq!(delta["payload"]["dirty"][0], "size");
        let state = session.debug_state(false);
        assert_eq!(state["display"]["block.out"]["stats"]["bounds"][1][0], 3.0);
        assert!(state["seq"].as_u64().unwrap() >= 1);

        // Place + connect through the round-trip table.
        session.handle(
            id,
            Some("pl".into()),
            ClientMessage::PlaceNode {
                func: "sphere".into(),
                cell: Some([40, 2]),
                connect: Some(crate::protocol::ConnectSpec {
                    from: WireEnd {
                        node: "size".into(),
                        port: "out".into(),
                    },
                    to_port: "radius".into(),
                    lift: false,
                }),
            },
        );
        session.wait_idle();
        let text = std::fs::read_to_string(&pipeline).unwrap();
        assert!(text.contains("sphere_1 = sphere(radius=size)"), "{text}");
        let sidecar = std::fs::read_to_string(Sidecar::path_for(&pipeline)).unwrap();
        assert!(sidecar.contains("\"sphere_1\""));
        let state = session.debug_state(false);
        assert!(
            state["display"]["sphere_1.out"]["stats"]["triangles"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(state["graph"]["nodes"].as_array().unwrap().len(), 4);

        // Unwire, rename, delete: text and downstream reds.
        session.handle(
            id,
            None,
            ClientMessage::Disconnect {
                to: WireEnd {
                    node: "sphere_1".into(),
                    port: "radius".into(),
                },
            },
        );
        session.wait_idle();
        let text = std::fs::read_to_string(&pipeline).unwrap();
        assert!(text.contains("sphere_1 = sphere()"), "{text}");
        let state = session.debug_state(false);
        assert_eq!(
            state["statuses"]["sphere_1"]["state"], "red",
            "required port unwired"
        );
        session.handle(
            id,
            None,
            ClientMessage::Rename {
                node: "span".into(),
                new: "extent".into(),
            },
        );
        session.wait_idle();
        let text = std::fs::read_to_string(&pipeline).unwrap();
        assert!(
            text.contains("block = box(x=extent, y=extent, z=extent)"),
            "{text}"
        );
        session.handle(
            id,
            None,
            ClientMessage::DeleteNode {
                node: "extent".into(),
            },
        );
        session.wait_idle();
        let state = session.debug_state(false);
        assert_eq!(
            state["statuses"]["block"]["state"], "red",
            "downstream red, never cascade"
        );
        assert!(
            state["text"]
                .as_str()
                .unwrap()
                .contains("block = box(x=extent")
        );
        // The box's frame is kept (last coherent value) — display still lists it.
        assert!(state["display"]["block.out"].is_object());
    }

    #[test]
    fn observers_cannot_write_and_lease_transfers() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx1, mut rx1) = unbounded_channel();
        let (tx2, mut rx2) = unbounded_channel();
        let (w, role_w) = session.connect(tx1);
        let (o, role_o) = session.connect(tx2);
        assert_eq!((role_w, role_o), (Role::Writer, Role::Observer));
        session.handle(
            o,
            Some("x".into()),
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        let msgs = texts(&drain(&mut rx2));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "lease");
        assert_eq!(error["payload"]["intent_id"], "x");
        session.disconnect(w);
        drain(&mut rx1);
        session.transfer_lease_if_free();
        let msgs = texts(&drain(&mut rx2));
        let lease = msgs.iter().rfind(|m| m["type"] == "lease").unwrap();
        assert_eq!(lease["payload"]["role"], "writer");
        session.handle(
            o,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        assert!(
            session.debug_state(false)["text"]
                .as_str()
                .unwrap()
                .contains("a = 2.0")
        );
    }

    #[test]
    fn probe_wire_reports_ok_lift_and_blocked_from_the_checker() {
        let (_dir, config) = project(
            "# cicada 1\n\
             c = circle(radius=2.0)\n\
             d = divide_curve(curve=c, count=8)\n\
             up = unit_z()\n\
             m = move(geometry=c, motion=up)\n\
             m2 = move(geometry=m, motion=up)\n\
             n = 3.0\n",
        );
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            Some("q".into()),
            ClientMessage::ProbeWire {
                from: WireEnd {
                    node: "d".into(),
                    port: "points".into(),
                },
            },
        );
        let msgs = texts(&drain(&mut rx));
        let probe = msgs.iter().find(|m| m["type"] == "wire_probe").unwrap();
        let targets = probe["payload"]["targets"].as_array().unwrap();
        let find = |node: &str, port: &str| {
            targets
                .iter()
                .find(|t| t["node"] == node && t["port"] == port)
                .unwrap()["verdict"]
                .clone()
        };
        assert_eq!(
            find("m", "geometry"),
            "lift",
            "[Point] into a T port → each()"
        );
        assert_eq!(find("m", "motion"), "blocked", "[Point] into a Vector port");
        assert_eq!(find("c", "radius"), "blocked");
        let catalog = probe["payload"]["catalog"].as_array().unwrap();
        let polyline = catalog.iter().find(|c| c["func"] == "polyline").unwrap();
        assert!(
            polyline["ports"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p[0] == "vertices" && p[1] == "ok")
        );
        assert!(
            !catalog.iter().any(|c| c["func"] == "add"),
            "add takes Numbers only"
        );
        // A cycle is blocked even though the types fit.
        session.handle(
            id,
            None,
            ClientMessage::ProbeWire {
                from: WireEnd {
                    node: "m2".into(),
                    port: "out".into(),
                },
            },
        );
        let msgs = texts(&drain(&mut rx));
        let probe = msgs.iter().find(|m| m["type"] == "wire_probe").unwrap();
        // m2 depends on m; wiring m2 into m.geometry closes the loop.
        let into_c = probe["payload"]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["node"] == "m" && t["port"] == "geometry")
            .unwrap();
        assert_eq!(into_c["verdict"], "blocked");
        assert!(
            into_c["reason"].as_str().unwrap().contains("cycle"),
            "{into_c}"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one refusal story, every refused gesture in turn
    fn blocked_wires_bad_params_and_shadowing_renames_are_refused_before_the_text_moves() {
        let (_dir, config) = project(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             dir = unit_x()\n\
             span = construct_domain(start=0.0, end=size)\n\
             xs = series(count=3)\n\
             sum = add(a=1.0, b=2.0)\n",
        );
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        let before = std::fs::read_to_string(&pipeline).unwrap();
        let refuse =
            |message: ClientMessage, rx: &mut tokio::sync::mpsc::UnboundedReceiver<Outgoing>| {
                session.handle(id, Some("x".into()), message);
                session.wait_idle();
                let msgs = texts(&drain(rx));
                let error = msgs
                    .iter()
                    .find(|m| m["type"] == "error")
                    .expect("an error message");
                assert_eq!(error["payload"]["intent_id"], "x");
                error["payload"]["message"].as_str().unwrap().to_owned()
            };
        // Vector into a Number port: blocked by the checker.
        let why = refuse(
            ClientMessage::Connect {
                from: WireEnd {
                    node: "dir".into(),
                    port: "out".into(),
                },
                to: WireEnd {
                    node: "span".into(),
                    port: "end".into(),
                },
                lift: false,
            },
            &mut rx,
        );
        assert!(why.contains("blocked"), "{why}");
        // A port the node does not have.
        let why = refuse(
            ClientMessage::Connect {
                from: WireEnd {
                    node: "size".into(),
                    port: "out".into(),
                },
                to: WireEnd {
                    node: "span".into(),
                    port: "radius".into(),
                },
                lift: false,
            },
            &mut rx,
        );
        assert!(why.contains("no port `radius`"), "{why}");
        // [Number] into a Number port without accepting the lift chip.
        let why = refuse(
            ClientMessage::Connect {
                from: WireEnd {
                    node: "xs".into(),
                    port: "out".into(),
                },
                to: WireEnd {
                    node: "sum".into(),
                    port: "a".into(),
                },
                lift: false,
            },
            &mut rx,
        );
        assert!(why.contains("lift"), "{why}");
        // A param edit that is not a literal.
        let why = refuse(
            ClientMessage::SetParam {
                node: "size".into(),
                port: Some("value".into()),
                value: "span".into(),
            },
            &mut rx,
        );
        assert!(why.contains("not a literal"), "{why}");
        // A rename onto a catalog name.
        let why = refuse(
            ClientMessage::Rename {
                node: "sum".into(),
                new: "add".into(),
            },
            &mut rx,
        );
        assert!(why.contains("catalog"), "{why}");
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            before,
            "nothing refused ever touched the text"
        );
        assert_eq!(
            session.debug_state(false)["text"].as_str().unwrap(),
            before,
            "and nothing lingered in memory"
        );
        // The same [Number] wire WITH the lift accepted goes through as each().
        session.handle(
            id,
            None,
            ClientMessage::Connect {
                from: WireEnd {
                    node: "xs".into(),
                    port: "out".into(),
                },
                to: WireEnd {
                    node: "sum".into(),
                    port: "a".into(),
                },
                lift: true,
            },
        );
        session.wait_idle();
        assert!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .contains("sum = add(a=each(xs), b=2.0)")
        );
        // Preview default: an override equal to the default is no override.
        session.handle(
            id,
            None,
            ClientMessage::SetPreview {
                node: "span".into(),
                on: Some(false),
            },
        );
        session.wait_idle();
        assert!(
            !Sidecar::path_for(&pipeline).exists()
                || !std::fs::read_to_string(Sidecar::path_for(&pipeline))
                    .unwrap()
                    .contains("preview"),
            "a Domain output is not displayable: preview=false IS the default, so no sidecar entry"
        );
    }

    #[test]
    fn external_edit_reloads_with_a_barrier_snapshot() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let _client = session.connect(tx);
        std::fs::write(&pipeline, "# cicada 1\na = 1.0\nb = 5.0\n").unwrap();
        assert!(session.reload_from_disk("test", false).unwrap());
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let snapshot = msgs.iter().find(|m| m["type"] == "snapshot").unwrap();
        assert_eq!(snapshot["payload"]["barrier"], true);
        assert_eq!(
            snapshot["payload"]["graph"]["nodes"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(
            !session.reload_from_disk("again", false).unwrap(),
            "unchanged → no-op"
        );
    }

    #[test]
    fn a_write_hold_keeps_intents_out_until_the_held_reload_lands() {
        // The git-revert shape (http.rs `api_git_revert`): hold writes,
        // put HEAD's bytes on disk, reload under the same hold. An intent
        // arriving in the window must wait and then apply to the REVERTED
        // text — without the hold it would persist its own text over the
        // restored file, the reload would find disk == memory and do
        // nothing, and the revert would be silently lost.
        let head = "# cicada 1\nsize = slider(value=2.0, min=0.5, max=5.0)\n";
        let (_dir, config) = project(head);
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            Some("s1".into()),
            ClientMessage::SetParam {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        assert!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .contains("value=3.0")
        );
        let _ = drain(&mut rx);

        let hold = session.hold_writes();
        std::fs::write(&pipeline, head).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let racer = {
            let session = Arc::clone(&session);
            std::thread::spawn(move || {
                session.handle(
                    id,
                    Some("s2".into()),
                    ClientMessage::SetParam {
                        node: "size".into(),
                        port: Some("value".into()),
                        value: "4.0".into(),
                    },
                );
                let _ = done_tx.send(());
            })
        };
        // Bounded wait in the safe direction: under the hold the intent
        // CANNOT complete, whatever the scheduling — the timeout only
        // bounds how long this test looks.
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "an intent landed while writes were held"
        );
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            head,
            "the restored bytes are untouched under the hold"
        );
        let reloaded = session
            .reload_from_disk_held(hold, "git revert", false)
            .unwrap();
        assert!(
            reloaded,
            "the reload saw the restored bytes, not an echo of the session's own write"
        );
        racer.join().unwrap();
        session.wait_idle();
        // The late intent applied to the reverted text: one op on top of
        // the barrier, and on the wire the barrier precedes its delta.
        assert!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .contains("value=4.0")
        );
        let state = session.debug_state(false);
        assert_eq!(state["history"]["depth"], 1);
        let msgs = texts(&drain(&mut rx));
        let barrier_at = msgs
            .iter()
            .position(|m| m["type"] == "snapshot" && m["payload"]["barrier"] == true)
            .expect("the barrier snapshot");
        let delta_at = msgs
            .iter()
            .position(|m| m["type"] == "delta" && m["payload"]["source"]["intent_id"] == "s2")
            .expect("the late intent's delta");
        assert!(barrier_at < delta_at, "{msgs:?}");
        assert_eq!(msgs[barrier_at]["payload"]["reason"], "git revert");
        assert_eq!(msgs[barrier_at]["payload"]["text"], head);
    }

    #[test]
    fn cancelled_generation_paints_nothing_and_keeps_the_last_coherent_display() {
        // doc 04: a cancelled solve leaves the LAST COHERENT frame. A
        // cancelled generation's half-finished outputs must not be painted
        // over the previous generation's (an incoherent mix) — and not
        // encoding them is what keeps Esc → idle fast (stage 6: ~100 MB of
        // half-done wall cutters used to sit between cancel() and idle).
        // Deterministic: the completion hook is driven directly with a
        // report marked cancelled.
        let (_dir, config) = project(
            "# cicada 1
             size = slider(value=2.0, min=0.5, max=5.0)
             span = construct_domain(start=0.0, end=size)
             block = box(x=span, y=span, z=span)
",
        );
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let before = session.debug_state(false);
        let shown_before = before["display"]["block.out"]["generation"]
            .as_u64()
            .expect("the box is displayed");
        let (lowered, generation) = {
            let inner = session.core.lock_inner();
            (
                Arc::clone(&inner.lowered),
                inner.last_complete.as_ref().map_or(0, |k| k.generation),
            )
        };
        let targets = all_targets(&lowered);
        let job = Job {
            lowered: Arc::clone(&lowered),
            targets,
            kind: JobKind::Structural,
            submitted: Instant::now(),
        };
        let cancelled = generation + 1;
        session.core.on_start(cancelled, &job);
        // The cancelled generation DID finish the box — with a different
        // mesh (a bigger box), stored so it is paintable. Painting it would
        // be the incoherent mix the rule forbids.
        let bigger = cicada_geom::meshbuild::box_mesh(
            &cicada_core::spatial::Plane::world_xy(),
            cicada_core::scalar::Domain::new(0.0, 4.0),
            cicada_core::scalar::Domain::new(0.0, 4.0),
            cicada_core::scalar::Domain::new(0.0, 4.0),
            1e-6,
        )
        .unwrap();
        let bigger =
            cicada_core::value::HashedValue::new(cicada_core::value::ValueData::Mesh(bigger))
                .unwrap();
        session.core.scheduler.store().store_value(&bigger).unwrap();
        let block_id = lowered
            .graph
            .nodes()
            .iter()
            .position(|n| n.name == "block")
            .expect("block is lowered");
        let report = Arc::new(cicada_sched::SolveReport {
            generation: cancelled,
            cancelled: true,
            outcomes: lowered
                .graph
                .nodes()
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    if index == block_id {
                        cicada_sched::NodeOutcome::Computed {
                            outputs: vec![bigger.hash()],
                            elements: 1,
                            nanos: 1,
                        }
                    } else {
                        cicada_sched::NodeOutcome::Cancelled
                    }
                })
                .collect(),
        });
        session.core.on_complete(cancelled, &job, report);
        let after = session.debug_state(false);
        assert_eq!(
            after["display"]["block.out"]["generation"]
                .as_u64()
                .unwrap(),
            shown_before,
            "the display keeps the previous generation's frame"
        );
        let timing = after["timings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["generation"].as_u64() == Some(cancelled))
            .expect("the cancelled generation is recorded");
        assert_eq!(timing["cancelled"], true);
        assert_eq!(timing["frame_bytes"], 0, "nothing painted: {timing}");
    }

    #[test]
    fn a_self_write_does_not_echo_back_as_an_external_reload() {
        // The project watcher (http.rs) fires for the server's OWN writes
        // too. reload_from_disk must recognise them and return Ok(false):
        // an Ok(true) reschedules a structural solve, and a watcher that
        // reloads every gesture-write churns forever (a fresh node never
        // settles to "done" — a Linux-CI Playwright flake, stage 6). The
        // guard covers BOTH the .cic and the sidecar.
        let (_dir, config) = project(
            "# cicada 1
             size = slider(value=2.0, min=0.5, max=5.0)
             span = construct_domain(start=0.0, end=size)
             block = box(x=span, y=span, z=span)
",
        );
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, _rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        // A structural gesture: it writes the .cic (and the sidecar).
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        // Now the watcher fires for that write. It must be a no-op.
        assert!(
            !session.reload_from_disk("watcher echo", false).unwrap(),
            "the server's own write must not reload (that would storm)"
        );
        // A sidecar-only gesture (move) then a watcher echo: also a no-op.
        session.handle(
            id,
            None,
            ClientMessage::MoveNode {
                node: "block".into(),
                cell: Some([5, 3]),
            },
        );
        session.wait_idle();
        assert!(
            !session
                .reload_from_disk("watcher echo (sidecar)", false)
                .unwrap(),
            "the server's own sidecar write must not reload either"
        );
        // A GENUINE external edit still reloads.
        std::fs::write(
            &session.core.config.pipeline,
            "# cicada 1
size = slider(value=4.0, min=0.5, max=5.0)
",
        )
        .unwrap();
        assert!(
            session.reload_from_disk("real edit", false).unwrap(),
            "a real external edit reloads"
        );
    }

    #[test]
    fn timings_carry_queue_wait_start_marks_and_the_esc_annotation() {
        // docs/15 measurement currency: every loop generation records when
        // it started, how long its job waited (accepted → start), and — only
        // when an Esc ended it — the server-side cancel → idle time.
        let (_dir, config) = project(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n",
        );
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, _rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            None,
            ClientMessage::ParamPreview {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        let state = session.debug_state(false);
        let timings = state["timings"].as_array().unwrap();
        let structural = timings
            .iter()
            .find(|t| t["kind"] == "structural")
            .expect("the load's generation is recorded");
        assert!(
            structural["started_ms"].as_f64().unwrap() > 0.0,
            "started_ms is read before the status board forgets the start: {structural}"
        );
        assert!(structural["queued_ms"].as_f64().unwrap() >= 0.0);
        assert!(structural["elapsed_ms"].as_f64().unwrap() > 0.0);
        assert!(
            structural.get("cancel_to_idle_ms").is_none(),
            "no Esc, no annotation: {structural}"
        );
        let preview = timings
            .iter()
            .find(|t| t["kind"] == "preview")
            .expect("the preview's generation is recorded");
        assert!(preview["queued_ms"].as_f64().unwrap() >= 0.0);
        assert!(preview["started_ms"].as_f64().unwrap() > 0.0);
        assert!(preview.get("cancel_to_idle_ms").is_none());

        // The loop's Esc hook annotates exactly the generation it names.
        let generation = preview["generation"].as_u64().unwrap();
        session
            .core
            .on_cancel_settled(generation, Duration::from_micros(12_500));
        let state = session.debug_state(false);
        let annotated: Vec<&serde_json::Value> = state["timings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t.get("cancel_to_idle_ms").is_some())
            .collect();
        assert_eq!(annotated.len(), 1, "{annotated:?}");
        assert_eq!(annotated[0]["generation"], generation);
        assert!((annotated[0]["cancel_to_idle_ms"].as_f64().unwrap() - 12.5).abs() < 1e-9);
        // An unknown generation (an errored one left no record) is a no-op.
        session
            .core
            .on_cancel_settled(999_999, Duration::from_millis(1));
        let state = session.debug_state(false);
        assert_eq!(
            state["timings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t.get("cancel_to_idle_ms").is_some())
                .count(),
            1
        );
    }

    // ------------------------------------------------------ undo/redo --

    /// The on-disk state of a pipeline: `(text, sidecar text or None)`.
    fn on_disk(pipeline: &Path) -> (String, Option<String>) {
        (
            std::fs::read_to_string(pipeline).unwrap(),
            std::fs::read_to_string(Sidecar::path_for(pipeline)).ok(),
        )
    }

    fn file_hash(path: &Path) -> String {
        blake3::hash(&std::fs::read(path).unwrap())
            .to_hex()
            .to_string()
    }

    fn history_of(session: &Session) -> serde_json::Value {
        serde_json::to_value(session.history()).unwrap()
    }

    fn project_with_clock(
        source: &str,
    ) -> (
        tempfile::TempDir,
        SessionConfig,
        Arc<cicada_sched::VirtualClock>,
    ) {
        let (dir, mut config) = project(source);
        let clock = Arc::new(cicada_sched::VirtualClock::new());
        config.op_clock = Some(clock.clone());
        (dir, config, clock)
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one story: five gestures, five undos, five redos, every state checked
    fn undo_and_redo_walk_the_recorded_states_byte_for_byte() {
        let (_dir, config, clock) = project_with_clock(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n",
        );
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        assert_eq!(
            history_of(&session),
            serde_json::json!({"can_undo": false, "can_redo": false, "undo_label": null,
                               "redo_label": null, "depth": 0})
        );

        // Five gestures, recording the state AFTER each (state 0 = initial).
        let mut states: Vec<(String, Option<String>)> = vec![on_disk(&pipeline)];
        let gestures: Vec<(ClientMessage, &str)> = vec![
            (
                ClientMessage::PlaceNode {
                    func: "sphere".into(),
                    cell: Some([40, 2]),
                    connect: None,
                },
                "place sphere",
            ),
            (
                ClientMessage::Connect {
                    from: WireEnd {
                        node: "size".into(),
                        port: "out".into(),
                    },
                    to: WireEnd {
                        node: "sphere_1".into(),
                        port: "radius".into(),
                    },
                    lift: false,
                },
                "wire size.out → sphere_1.radius",
            ),
            (
                ClientMessage::SetParam {
                    node: "size".into(),
                    port: Some("value".into()),
                    value: "3.0".into(),
                },
                "set size.value = 3.0",
            ),
            (
                ClientMessage::MoveNode {
                    node: "block".into(),
                    cell: Some([7, 7]),
                },
                "move block",
            ),
            (
                ClientMessage::DeleteNode {
                    node: "span".into(),
                },
                "delete span",
            ),
        ];
        for (step, (gesture, label)) in gestures.into_iter().enumerate() {
            clock.advance(10_000_000); // 10 ms
            session.handle(id, Some(format!("g{step}")), gesture);
            session.wait_idle();
            let msgs = texts(&drain(&mut rx));
            let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
            assert_eq!(deltas.len(), 1, "one delta per gesture: {msgs:?}");
            assert_eq!(deltas[0]["payload"]["source"]["label"], label);
            assert_eq!(deltas[0]["payload"]["history"]["depth"], step + 1);
            assert_eq!(deltas[0]["payload"]["history"]["undo_label"], label);
            assert_eq!(deltas[0]["payload"]["history"]["can_redo"], false);
            let disk = on_disk(&pipeline);
            assert_eq!(
                deltas[0]["payload"]["text"], disk.0,
                "the delta's text IS the file"
            );
            states.push(disk);
        }
        assert_eq!(states.len(), 6);
        assert!(states[4].1.is_some(), "the move wrote a sidecar");
        assert!(
            !states[5].0.contains("span ="),
            "span is gone: {}",
            states[5].0
        );
        let debug = session.debug_state(false);
        let ops = debug["ops"].as_array().unwrap();
        assert_eq!(ops.len(), 5);
        assert_eq!(
            ops.iter()
                .map(|o| o["id"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(
            ops.iter()
                .map(|o| o["at"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![10, 20, 30, 40, 50],
            "op timestamps come from the injected clock (ms)"
        );
        assert!(
            ops.iter()
                .all(|o| o["actor"] == serde_json::json!({"kind": "human"}))
        );

        // Undo ×5: after the k-th undo the state is states[5 - k], in
        // memory AND on disk, byte for byte.
        for k in 1..=5 {
            session.handle(id, Some(format!("u{k}")), ClientMessage::Undo {});
            session.wait_idle();
            let msgs = texts(&drain(&mut rx));
            let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
            assert_eq!(deltas.len(), 1, "undo {k}: one delta: {msgs:?}");
            let expected = &states[5 - k];
            assert_eq!(
                deltas[0]["payload"]["text"], expected.0,
                "undo {k}: the broadcast text is the recorded state"
            );
            assert!(
                deltas[0]["payload"]["source"]["label"]
                    .as_str()
                    .unwrap()
                    .starts_with("undo: "),
                "{}",
                deltas[0]["payload"]["source"]["label"]
            );
            assert_eq!(deltas[0]["payload"]["history"]["depth"], 5 - k);
            assert_eq!(deltas[0]["payload"]["history"]["can_redo"], true);
            assert_eq!(
                on_disk(&pipeline),
                *expected,
                "undo {k}: disk (text + sidecar) is the recorded state"
            );
            assert_eq!(
                session.debug_state(false)["text"].as_str().unwrap(),
                expected.0,
                "undo {k}: memory is the recorded state"
            );
            if k == 1 {
                // Undo never recomputes: the un-deleted span's cone was
                // solved two ops ago, so the restored block is a memo hit.
                let statuses = &session.debug_state(false)["statuses"];
                assert_eq!(statuses["block"]["state"], "cached", "{statuses}");
                assert_eq!(statuses["span"]["state"], "cached", "{statuses}");
            }
        }
        assert_eq!(
            session.debug_state(false)["text"].as_str().unwrap(),
            states[0].0,
            "five undos → the initial text"
        );
        // One more undo: refused, and says why (empty, not barrier).
        session.handle(id, Some("u6".into()), ClientMessage::Undo {});
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "nothing_to_undo");
        assert_eq!(error["payload"]["intent_id"], "u6");
        assert!(
            error["payload"]["message"]
                .as_str()
                .unwrap()
                .contains("already undone"),
            "{}",
            error["payload"]["message"]
        );
        assert!(
            !msgs.iter().any(|m| m["type"] == "delta"),
            "a refused undo broadcasts nothing"
        );

        // Redo ×5: after the k-th redo the state is states[k].
        for (k, expected) in states.iter().enumerate().skip(1) {
            session.handle(id, Some(format!("r{k}")), ClientMessage::Redo {});
            session.wait_idle();
            let msgs = texts(&drain(&mut rx));
            let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
            assert_eq!(deltas.len(), 1, "redo {k}: one delta");
            assert_eq!(deltas[0]["payload"]["text"], expected.0);
            assert!(
                deltas[0]["payload"]["source"]["label"]
                    .as_str()
                    .unwrap()
                    .starts_with("redo: ")
            );
            assert_eq!(deltas[0]["payload"]["history"]["depth"], k);
            assert_eq!(on_disk(&pipeline), *expected, "redo {k}: disk matches");
        }
        session.handle(id, Some("r6".into()), ClientMessage::Redo {});
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "nothing_to_redo");
        assert_eq!(
            history_of(&session),
            serde_json::json!({"can_undo": true, "can_redo": false, "undo_label": "delete span",
                               "redo_label": null, "depth": 5})
        );
        // The restored final state solves to the same display as before
        // (undo never recomputes — warm keys): the sphere is back and the
        // block's box is gone (span was deleted → block is red).
        let state = session.debug_state(false);
        assert_eq!(state["statuses"]["block"]["state"], "red");
        assert!(state["display"]["sphere_1.out"].is_object());

        // A new op after undoing two truncates the redo tail.
        session.handle(id, None, ClientMessage::Undo {});
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        drain(&mut rx);
        assert_eq!(history_of(&session)["can_redo"], true);
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "size".into(),
                port: Some("value".into()),
                value: "4.0".into(),
            },
        );
        session.wait_idle();
        let history = history_of(&session);
        assert_eq!(history["can_redo"], false, "{history}");
        assert_eq!(history["depth"], 4);
        assert_eq!(history["undo_label"], "set size.value = 4.0");
        let ops = session.debug_state(false)["ops"].as_array().unwrap().len();
        assert_eq!(ops, 4, "the two undone ops were dropped from the log");
    }

    #[test]
    fn undo_and_redo_are_lease_gated_like_every_write() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx1, mut rx1) = unbounded_channel();
        let (tx2, mut rx2) = unbounded_channel();
        let (w, _) = session.connect(tx1);
        let (o, _) = session.connect(tx2);
        session.handle(
            w,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        drain(&mut rx1);
        drain(&mut rx2);
        for (intent, message) in [("u", ClientMessage::Undo {}), ("r", ClientMessage::Redo {})] {
            session.handle(o, Some(intent.into()), message);
            let msgs = texts(&drain(&mut rx2));
            let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
            assert_eq!(error["payload"]["kind"], "lease", "{intent}");
            assert_eq!(error["payload"]["intent_id"], intent);
        }
        assert!(
            session.debug_state(false)["text"]
                .as_str()
                .unwrap()
                .contains("a = 2.0"),
            "the observer's undo changed nothing"
        );
        assert_eq!(history_of(&session)["depth"], 1);
        // The writer's undo goes through — and the observer sees the delta.
        session.handle(w, None, ClientMessage::Undo {});
        session.wait_idle();
        let msgs = texts(&drain(&mut rx2));
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert!(
            delta["payload"]["text"]
                .as_str()
                .unwrap()
                .contains("a = 1.0")
        );
        assert_eq!(delta["payload"]["source"]["label"], "undo: set a = 2.0");
    }

    #[test]
    fn param_previews_and_effectful_runs_are_never_ops() {
        let dir = tempfile::tempdir().unwrap();
        let obj = dir.path().join("out.obj");
        let obj_text = obj.to_string_lossy().replace('\\', "/");
        let source = format!(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n\
             dx = unit_x()\n\
             blocks = linear_array(geometry=block, direction=dx, count=2)\n\
             dump = export_obj(meshes=blocks, path=\"{obj_text}\")\n"
        );
        let pipeline = dir.path().join("p.cic");
        std::fs::write(&pipeline, &source).unwrap();
        let config = SessionConfig {
            project_dir: dir.path().to_owned(),
            pipeline: pipeline.clone(),
            cache_dir: Some(dir.path().join("cache")),
            threads: 2,
            project: ProjectConfig::default(),
            op_clock: None,
        };
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            None,
            ClientMessage::ParamPreview {
                node: "size".into(),
                port: Some("value".into()),
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        assert_eq!(history_of(&session)["depth"], 0, "a preview is not an op");
        assert!(!texts(&drain(&mut rx)).iter().any(|m| m["type"] == "delta"));
        session.run_effectful("dump").expect("the export runs");
        assert!(obj.exists(), "the exporter wrote its file");
        assert_eq!(
            history_of(&session),
            serde_json::json!({"can_undo": false, "can_redo": false, "undo_label": null,
                               "redo_label": null, "depth": 0}),
            "an effectful run is not an op (non-undoable, and says so by its absence)"
        );
        assert_eq!(
            session.debug_state(false)["ops"].as_array().unwrap().len(),
            0
        );
        let msgs = texts(&drain(&mut rx));
        assert!(msgs.iter().any(|m| m["type"] == "run_finished"));
        assert!(!msgs.iter().any(|m| m["type"] == "delta"));
        // The text never moved: nothing to undo, and the file is the source.
        assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), source);
    }

    #[test]
    fn the_op_log_keeps_the_last_200_ops() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        for i in 0..(OP_LOG_CAP + 5) {
            session.handle(
                id,
                None,
                ClientMessage::SetParam {
                    node: "a".into(),
                    port: None,
                    value: format!("{}.0", i + 2),
                },
            );
            drain(&mut rx);
        }
        session.wait_idle();
        let state = session.debug_state(false);
        let ops = state["ops"].as_array().unwrap();
        assert_eq!(ops.len(), OP_LOG_CAP);
        assert_eq!(ops[0]["id"], 6, "the five oldest dropped off");
        assert_eq!(ops[OP_LOG_CAP - 1]["id"], OP_LOG_CAP + 5);
        assert_eq!(state["history"]["depth"], OP_LOG_CAP);
        // Undo all 200: back to the oldest KEPT op's before-state — op 6
        // set 7.0, so its `before` is the 6.0 that op 5 wrote.
        for _ in 0..OP_LOG_CAP {
            session.handle(id, None, ClientMessage::Undo {});
            drain(&mut rx);
        }
        session.wait_idle();
        assert!(
            session.debug_state(false)["text"]
                .as_str()
                .unwrap()
                .contains("a = 6.0"),
            "{}",
            session.debug_state(false)["text"]
        );
        session.handle(id, Some("x".into()), ClientMessage::Undo {});
        let msgs = texts(&drain(&mut rx));
        assert_eq!(
            msgs.iter().find(|m| m["type"] == "error").unwrap()["payload"]["kind"],
            "nothing_to_undo"
        );
    }

    #[test]
    fn an_external_edit_is_the_barrier_that_clears_the_log() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        assert_eq!(history_of(&session)["depth"], 1);
        drain(&mut rx);
        // Someone edits the file outside the canvas (git checkout, editor).
        std::fs::write(&pipeline, "# cicada 1\na = 2.0\nb = 5.0\n").unwrap();
        assert!(session.reload_from_disk("test", false).unwrap());
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let snapshot = msgs.iter().find(|m| m["type"] == "snapshot").unwrap();
        assert_eq!(snapshot["payload"]["barrier"], true);
        assert_eq!(
            snapshot["payload"]["history"],
            serde_json::json!({"can_undo": false, "can_redo": false, "undo_label": null,
                               "redo_label": null, "depth": 0}),
            "the barrier snapshot carries the cleared history"
        );
        session.handle(id, Some("u".into()), ClientMessage::Undo {});
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "nothing_to_undo");
        assert!(
            error["payload"]["message"]
                .as_str()
                .unwrap()
                .contains("reload barrier"),
            "the refusal names the barrier: {}",
            error["payload"]["message"]
        );
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            "# cicada 1\na = 2.0\nb = 5.0\n",
            "nothing was restored over the external edit"
        );
        // Life goes on: a new op after the barrier is undoable again, and
        // the message no longer blames the barrier once the log has ops.
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "b".into(),
                port: None,
                value: "6.0".into(),
            },
        );
        session.wait_idle();
        assert_eq!(history_of(&session)["depth"], 1);
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        drain(&mut rx);
        session.handle(id, Some("u2".into()), ClientMessage::Undo {});
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert!(
            !error["payload"]["message"]
                .as_str()
                .unwrap()
                .contains("barrier"),
            "{}",
            error["payload"]["message"]
        );
    }

    // ---------------------------------------------------------- batch --

    #[test]
    #[allow(clippy::too_many_lines)] // the all-or-nothing story: success, failure, validation
    fn a_batch_is_one_op_one_delta_all_or_nothing() {
        let (_dir, config) = project(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n\
             n = 3.0\n",
        );
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);

        // A multi-move + a param + a delete as ONE op.
        session.handle(
            id,
            Some("b1".into()),
            ClientMessage::Batch {
                label: "move 2 nodes, set n, delete span".into(),
                ops: vec![
                    ClientMessage::MoveNode {
                        node: "size".into(),
                        cell: Some([1, 1]),
                    },
                    ClientMessage::MoveNode {
                        node: "block".into(),
                        cell: Some([9, 1]),
                    },
                    ClientMessage::SetParam {
                        node: "n".into(),
                        port: None,
                        value: "4.0".into(),
                    },
                    ClientMessage::DeleteNode {
                        node: "span".into(),
                    },
                ],
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
        assert_eq!(deltas.len(), 1, "one delta for the whole batch: {msgs:?}");
        assert_eq!(deltas[0]["payload"]["source"]["intent_id"], "b1");
        assert_eq!(
            deltas[0]["payload"]["source"]["label"],
            "move 2 nodes, set n, delete span"
        );
        assert_eq!(deltas[0]["payload"]["history"]["depth"], 1);
        let dirty = deltas[0]["payload"]["dirty"].as_array().unwrap();
        assert!(dirty.iter().any(|d| d == "n"), "{dirty:?}");
        assert!(
            dirty.iter().any(|d| d == "block"),
            "delete's dependents: {dirty:?}"
        );
        let (text, sidecar) = on_disk(&pipeline);
        assert!(text.contains("n = 4.0"), "{text}");
        assert!(!text.contains("span ="), "{text}");
        let sidecar = sidecar.expect("moves wrote the sidecar");
        assert!(sidecar.contains("\"size\"") && sidecar.contains("\"block\""));
        assert_eq!(
            session.debug_state(false)["ops"].as_array().unwrap().len(),
            1
        );
        let after_batch = on_disk(&pipeline);

        // A batch whose THIRD op fails (unknown node) changes nothing: text,
        // sidecar, disk and history are exactly as before; no delta.
        session.handle(
            id,
            Some("b2".into()),
            ClientMessage::Batch {
                label: "doomed".into(),
                ops: vec![
                    ClientMessage::MoveNode {
                        node: "size".into(),
                        cell: Some([2, 2]),
                    },
                    ClientMessage::SetParam {
                        node: "n".into(),
                        port: None,
                        value: "5.0".into(),
                    },
                    ClientMessage::DeleteNode {
                        node: "nope".into(),
                    },
                ],
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        assert!(
            !msgs.iter().any(|m| m["type"] == "delta"),
            "a failed batch broadcasts no delta: {msgs:?}"
        );
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["intent_id"], "b2");
        assert_eq!(error["payload"]["index"], 2, "names the failing op");
        assert!(
            error["payload"]["message"]
                .as_str()
                .unwrap()
                .contains("op 2 (delete_node)"),
            "{}",
            error["payload"]["message"]
        );
        assert_eq!(on_disk(&pipeline), after_batch, "disk untouched");
        assert_eq!(
            session.debug_state(false)["text"].as_str().unwrap(),
            after_batch.0,
            "memory untouched"
        );
        assert_eq!(
            session.core.lock_inner().sidecar.overrides["size"].cell,
            Some([1, 1]),
            "the first op's move was rolled back in memory"
        );
        assert_eq!(history_of(&session)["depth"], 1, "no op recorded");

        // Undo of the batch restores all four gestures at once.
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        let (text, sidecar) = on_disk(&pipeline);
        assert!(
            text.contains("n = 3.0") && text.contains("span ="),
            "{text}"
        );
        assert!(sidecar.is_none(), "the moves are gone → no sidecar file");

        // Validation: a non-gesture element and an empty batch are refused
        // before anything is touched.
        session.handle(
            id,
            Some("b3".into()),
            ClientMessage::Batch {
                label: "bad".into(),
                ops: vec![
                    ClientMessage::MoveNode {
                        node: "size".into(),
                        cell: None,
                    },
                    ClientMessage::Undo {},
                ],
            },
        );
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "protocol");
        assert_eq!(error["payload"]["index"], 1);
        session.handle(
            id,
            Some("b4".into()),
            ClientMessage::Batch {
                label: "empty".into(),
                ops: vec![],
            },
        );
        let msgs = texts(&drain(&mut rx));
        assert_eq!(
            msgs.iter().find(|m| m["type"] == "error").unwrap()["payload"]["kind"],
            "protocol"
        );
        // A place + connect-to-it + move-it in one batch: later elements
        // see the earlier ones.
        session.handle(
            id,
            Some("b5".into()),
            ClientMessage::Batch {
                label: "add a sphere".into(),
                ops: vec![
                    ClientMessage::PlaceNode {
                        func: "sphere".into(),
                        cell: None,
                        connect: None,
                    },
                    ClientMessage::Connect {
                        from: WireEnd {
                            node: "size".into(),
                            port: "out".into(),
                        },
                        to: WireEnd {
                            node: "sphere_1".into(),
                            port: "radius".into(),
                        },
                        lift: false,
                    },
                    ClientMessage::MoveNode {
                        node: "sphere_1".into(),
                        cell: Some([3, 3]),
                    },
                ],
            },
        );
        session.wait_idle();
        let (text, sidecar) = on_disk(&pipeline);
        assert!(text.contains("sphere_1 = sphere(radius=size)"), "{text}");
        assert!(sidecar.unwrap().contains("\"sphere_1\""));
        assert_eq!(
            session.debug_state(false)["statuses"]["sphere_1"]["state"],
            "done"
        );
    }

    // ----------------------------------------------------- apply_text --

    fn apply(
        session: &Session,
        base: &str,
        files: Vec<(&str, &str)>,
        label: &str,
    ) -> Result<serde_json::Value, IntentError> {
        session.apply_text(
            &ApplyTextRequest {
                base_text_hash: base.to_owned(),
                files: files
                    .into_iter()
                    .map(|(path, text)| crate::protocol::FileText {
                        path: path.to_owned(),
                        text: text.to_owned(),
                    })
                    .collect(),
                label: label.to_owned(),
                actor: Actor::Agent {
                    prompt: Some("test".to_owned()),
                },
            },
            DeltaSource {
                client: None,
                intent_id: None,
                label: label.to_owned(),
            },
        )
    }

    #[test]
    #[allow(clippy::too_many_lines)] // happy path, stale base, parse error, bad paths — one fixture
    fn apply_text_is_atomic_against_a_base_hash() {
        let (_dir, config) = project(
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)\n\
             block = box(x=span, y=span, z=span)\n",
        );
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        assert_eq!(session.relative(), "p.cic");
        let base = session.edit_text();
        assert_eq!(base["path"], "p.cic");
        assert_eq!(base["text"], std::fs::read_to_string(&pipeline).unwrap());
        assert_eq!(
            base["text_hash"],
            file_hash(&pipeline),
            "the hash IS the file's"
        );
        let base_hash = base["text_hash"].as_str().unwrap().to_owned();

        // Happy path: a new binding appears; one delta; depth + 1.
        let new_text = format!(
            "{}ball = sphere(radius=size)\n",
            base["text"].as_str().unwrap()
        );
        let result = apply(
            &session,
            &base_hash,
            vec![("p.cic", &new_text)],
            "add a ball",
        )
        .unwrap();
        session.wait_idle();
        assert_eq!(result["ok"], true);
        assert_eq!(result["history"]["depth"], 1);
        assert_eq!(result["text_hash"], file_hash(&pipeline));
        assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), new_text);
        let msgs = texts(&drain(&mut rx));
        let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
        assert_eq!(deltas.len(), 1, "{msgs:?}");
        assert_eq!(deltas[0]["payload"]["source"]["label"], "add a ball");
        assert_eq!(
            deltas[0]["payload"]["source"]["client"],
            serde_json::Value::Null
        );
        assert_eq!(deltas[0]["payload"]["dirty"], serde_json::json!(["ball"]));
        assert_eq!(deltas[0]["payload"]["history"]["undo_label"], "add a ball");
        let state = session.debug_state(false);
        assert_eq!(state["statuses"]["ball"]["state"], "done");
        assert_eq!(
            state["ops"][0]["actor"],
            serde_json::json!({"kind": "agent", "prompt": "test"})
        );
        let hash_after = state["text_hash"].as_str().unwrap().to_owned();

        // Stale base: refused with the current hash; disk untouched.
        let error = apply(
            &session,
            &base_hash,
            vec![("p.cic", "# cicada 1\nx = 1.0\n")],
            "stale",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "stale_base");
        assert_eq!(
            error.details()["current_text_hash"],
            hash_after,
            "the refusal carries the hash to rebase on"
        );
        assert_eq!(file_hash(&pipeline), hash_after, "disk untouched");
        assert_eq!(history_of(&session)["depth"], 1);
        assert!(!texts(&drain(&mut rx)).iter().any(|m| m["type"] == "delta"));

        // Parse error: refused with diagnostics; disk untouched. (Check
        // diagnostics — an unknown name — are NOT a refusal: red is a valid
        // state.)
        let error = apply(
            &session,
            &hash_after,
            vec![("p.cic", "# cicada 1\nx = (1.0\n")],
            "broken",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "parse_error");
        let diagnostics = error.details()["diagnostics"].as_array().unwrap().clone();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0]["span"]["line"], 2);
        assert_eq!(file_hash(&pipeline), hash_after);
        let error = apply(
            &session,
            &hash_after,
            vec![("p.cic", "a = 1.0\n")],
            "no pragma",
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            "parse_error",
            "a missing pragma is a parse problem"
        );
        let red = apply(
            &session,
            &hash_after,
            vec![("p.cic", "# cicada 1\nx = add(a=nope, b=1.0)\n")],
            "red is fine",
        )
        .unwrap();
        session.wait_idle();
        assert_eq!(red["history"]["depth"], 2);
        assert_eq!(session.debug_state(false)["statuses"]["x"]["state"], "red");
        let hash_red = red["text_hash"].as_str().unwrap().to_owned();
        drain(&mut rx);

        // Disallowed paths: anything but this pipeline, its sidecar, or
        // scripts/*.py beside it — refused before any write.
        for bad in [
            "other.cic",
            "../p.cic",
            "/p.cic",
            "C:/p.cic",
            "sub\\p.cic",
            "scripts/x.txt",
            "scripts/sub/x.py",
            "x.py",
            "p.cic.layout.json.bak",
            "",
        ] {
            let error = apply(&session, &hash_red, vec![(bad, "")], "bad path").unwrap_err();
            assert_eq!(error.kind(), "path_not_allowed", "`{bad}`: {error}");
        }
        // A duplicate path is a protocol error; a sidecar that does not
        // parse is a parse error.
        let error = apply(
            &session,
            &hash_red,
            vec![("p.cic", "# cicada 1\n"), ("p.cic", "# cicada 1\n")],
            "dup",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "protocol");
        let error = apply(
            &session,
            &hash_red,
            vec![("p.cic.layout.json", "{not json")],
            "bad sidecar",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "parse_error");
        assert_eq!(
            file_hash(&pipeline),
            hash_red,
            "nothing refused touched the disk"
        );
        assert!(!Sidecar::path_for(&pipeline).exists());
        assert_eq!(history_of(&session)["depth"], 2);

        // Text + sidecar together; then undo restores both.
        let sidecar_text = Sidecar {
            overrides: [(
                "x".to_owned(),
                crate::sidecar::Override {
                    cell: Some([5, 5]),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }
        .render();
        apply(
            &session,
            &hash_red,
            vec![
                ("p.cic", "# cicada 1\nx = 2.0\n"),
                ("p.cic.layout.json", &sidecar_text),
            ],
            "text + layout",
        )
        .unwrap();
        session.wait_idle();
        assert_eq!(
            std::fs::read_to_string(Sidecar::path_for(&pipeline)).unwrap(),
            sidecar_text,
            "the sidecar bytes are written as given"
        );
        let nodes = session.debug_state(false)["graph"]["nodes"].clone();
        let x = nodes
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == "x")
            .unwrap()
            .clone();
        assert_eq!(
            x["cell"],
            serde_json::json!([5, 5]),
            "and the view reads them"
        );
        assert_eq!(history_of(&session)["depth"], 3);
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            "# cicada 1\nx = add(a=nope, b=1.0)\n"
        );
        assert!(
            !Sidecar::path_for(&pipeline).exists(),
            "the sidecar went back to empty → no file"
        );
        // The watcher's echo of our own writes is still a no-op.
        assert!(!session.reload_from_disk("echo", false).unwrap());
    }

    #[test]
    fn apply_text_restores_the_earlier_files_when_a_later_write_fails() {
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let _client = session.connect(tx);
        let before_hash = file_hash(&pipeline);
        let before_text = std::fs::read_to_string(&pipeline).unwrap();
        // Fault injection: the second target's TEMP path is a directory, so
        // its atomic write fails after the first file (the .cic) already
        // landed — the mid-way failure the rollback exists for.
        let sidecar_path = Sidecar::path_for(&pipeline);
        let sidecar_tmp = pipeline
            .parent()
            .unwrap()
            .join(".p.cic.layout.json.cicada-tmp");
        std::fs::create_dir(&sidecar_tmp).unwrap();
        let sidecar_text = Sidecar {
            overrides: [(
                "a".to_owned(),
                crate::sidecar::Override {
                    cell: Some([1, 1]),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }
        .render();
        let error = apply(
            &session,
            &before_hash,
            vec![
                ("p.cic", "# cicada 1\na = 2.0\n"),
                ("p.cic.layout.json", &sidecar_text),
            ],
            "two files",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "io_error");
        assert!(
            error
                .to_string()
                .contains("1 file(s) written before it were restored"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            before_text,
            "the .cic went back to its bytes"
        );
        assert_eq!(file_hash(&pipeline), before_hash);
        assert!(!sidecar_path.exists(), "the sidecar was never created");
        std::fs::remove_dir(&sidecar_tmp).unwrap();
        let leftovers: Vec<String> = std::fs::read_dir(pipeline.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("cicada-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp files linger: {leftovers:?}");
        let state = session.debug_state(false);
        assert_eq!(state["text"], before_text, "memory untouched");
        assert_eq!(state["text_hash"], before_hash);
        assert_eq!(state["history"]["depth"], 0, "no op recorded");
        assert!(
            !texts(&drain(&mut rx)).iter().any(|m| m["type"] == "delta"),
            "no delta"
        );
        // A target that IS a directory is refused before any write (its
        // pre-read fails) — loud io_error, disk and memory untouched.
        std::fs::create_dir(&sidecar_path).unwrap();
        let error = apply(
            &session,
            &before_hash,
            vec![
                ("p.cic", "# cicada 1\na = 2.0\n"),
                ("p.cic.layout.json", &sidecar_text),
            ],
            "dir target",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "io_error");
        assert_eq!(file_hash(&pipeline), before_hash, "nothing written");
        std::fs::remove_dir(&sidecar_path).unwrap();
        // The session is healthy afterwards: a normal apply still works
        // once the obstacles are gone.
        apply(
            &session,
            &before_hash,
            vec![("p.cic", "# cicada 1\na = 3.0\n")],
            "ok",
        )
        .unwrap();
        assert!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .contains("a = 3.0")
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // add a script node, echo guard, undo, a broken script — one fixture
    fn apply_text_with_a_script_reloads_the_catalog_without_clearing_the_log() {
        // Python 3 on PATH is a dev/CI requirement (as for scripts.rs).
        let (_dir, config) = project("# cicada 1\na = 2.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        // One human op first: it must survive the script reload.
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        drain(&mut rx);
        let base = session.edit_text()["text_hash"]
            .as_str()
            .unwrap()
            .to_owned();
        let script = "import cicada\n\n\
                      @cicada.node(title=\"Triple\", description=\"x times three.\")\n\
                      def triple(x: \"Number\") -> \"Number\":\n    return x * 3.0\n";
        assert!(
            !session.catalog_value()["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["name"] == "triple")
        );
        let result = apply(
            &session,
            &base,
            vec![
                ("scripts/triple.py", script),
                ("p.cic", "# cicada 1\na = 3.0\nt = triple(x=a)\n"),
            ],
            "add a script node",
        )
        .unwrap();
        session.wait_idle();
        assert_eq!(
            result["history"]["depth"], 2,
            "the earlier op is still there"
        );
        assert!(
            session.catalog_value()["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["name"] == "triple"),
            "the catalog reloaded with the new script node"
        );
        let msgs = texts(&drain(&mut rx));
        let snapshots: Vec<_> = msgs.iter().filter(|m| m["type"] == "snapshot").collect();
        assert_eq!(
            snapshots.len(),
            1,
            "a scripts change hydrates via ONE snapshot: {msgs:?}"
        );
        assert_eq!(snapshots[0]["payload"]["barrier"], false, "not a barrier");
        assert_eq!(snapshots[0]["payload"]["history"]["depth"], 2);
        assert!(!msgs.iter().any(|m| m["type"] == "delta"));
        let state = session.debug_state(false);
        assert_eq!(
            state["statuses"]["t"]["state"], "done",
            "{}",
            state["statuses"]
        );
        assert_eq!(state["ops"].as_array().unwrap().len(), 2);
        assert_eq!(
            std::fs::read_to_string(pipeline.parent().unwrap().join("scripts/triple.py")).unwrap(),
            script
        );
        // The watcher's rescan for our own script write is recognised as
        // an echo — NOT a barrier.
        assert!(
            !session
                .reload_from_disk("watcher echo (script)", true)
                .unwrap(),
            "our own script write must not reload (that would clear the log)"
        );
        assert_eq!(history_of(&session)["depth"], 2);
        // Undo restores the text (the script file stays: the snapshot is
        // text + sidecar by the ledger row); the op log still counts it.
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            "# cicada 1\na = 3.0\n"
        );
        assert_eq!(history_of(&session)["can_redo"], true);
        // A script that does not describe is a parse failure of the edit:
        // refused, the files restored (the broken script is gone again).
        let base = session.edit_text()["text_hash"]
            .as_str()
            .unwrap()
            .to_owned();
        let error = apply(
            &session,
            &base,
            vec![("scripts/broken.py", "def (:\n")],
            "broken script",
        )
        .unwrap_err();
        assert_eq!(error.kind(), "parse_error", "{error}");
        assert!(
            !pipeline
                .parent()
                .unwrap()
                .join("scripts/broken.py")
                .exists(),
            "the broken script was removed again"
        );
        assert!(
            pipeline
                .parent()
                .unwrap()
                .join("scripts/triple.py")
                .exists(),
            "the good one stays"
        );
        assert_eq!(history_of(&session)["depth"], 1);
    }

    // ------------------------------------------------ persist failures --

    /// The text the session holds and the hash it vouches for, as an
    /// agent sees them through `GET /api/edit/text`.
    fn text_and_hash(session: &Session) -> (String, String) {
        let value = session.edit_text();
        (
            value["text"].as_str().unwrap().to_owned(),
            value["text_hash"].as_str().unwrap().to_owned(),
        )
    }

    fn blake3_hex(text: &str) -> String {
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // a gesture, then an undo, each against a failing sidecar save
    fn a_failed_persist_leaves_disk_memory_and_the_hash_as_they_were() {
        // Review finding (2026-08-20): a commit whose `.cic` write landed
        // but whose sidecar save failed (a transient lock — os error 5/32,
        // exactly what Dropbox does to this project dir) left the REFUSED
        // text on disk with memory rolled back and `text_hash` vouching
        // for the disk bytes: `GET /api/edit/text` shipped a text whose
        // hash was not the one beside it, the stale-base check accepted an
        // edit based on a text never on disk, and the watcher saw nothing
        // to reconcile. The contract now: a failed persist restores the
        // disk, and `text_hash` is ALWAYS the hash of the text in memory.
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let sidecar_path = Sidecar::path_for(&pipeline);
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        // One good op first, so the undo path can be exercised too.
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        drain(&mut rx);
        let before = on_disk(&pipeline);
        assert_eq!(before.0, "# cicada 1\na = 2.0\n");
        let (memory, hash) = text_and_hash(&session);
        assert_eq!(memory, before.0);
        assert_eq!(hash, blake3_hex(&memory));

        // Fault injection: the sidecar path is a NON-EMPTY directory, so
        // neither writing nor removing the sidecar can succeed — the text
        // write before it does.
        std::fs::create_dir(&sidecar_path).unwrap();
        std::fs::write(sidecar_path.join("occupied"), b"x").unwrap();

        // A text gesture: the .cic lands, the sidecar save fails.
        session.handle(
            id,
            Some("g".into()),
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "3.0".into(),
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "persist", "{error}");
        assert_eq!(error["payload"]["intent_id"], "g");
        assert!(
            !msgs.iter().any(|m| m["type"] == "delta"),
            "a failed persist broadcasts no delta: {msgs:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            before.0,
            "the refused text was taken back off the disk"
        );
        let (memory, hash) = text_and_hash(&session);
        assert_eq!(memory, before.0, "memory rolled back");
        assert_eq!(
            hash,
            blake3_hex(&memory),
            "text_hash is the hash of the text it ships with"
        );
        assert_eq!(hash, file_hash(&pipeline), "…which is the file's");
        assert_eq!(history_of(&session)["depth"], 1, "no op recorded");

        // An undo against the same fault: the restore lands, the sidecar
        // save fails → everything (including the cursor) stays put.
        session.handle(id, Some("u".into()), ClientMessage::Undo {});
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "persist", "{error}");
        assert!(!msgs.iter().any(|m| m["type"] == "delta"));
        assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), before.0);
        let (memory, hash) = text_and_hash(&session);
        assert_eq!(memory, before.0);
        assert_eq!(hash, blake3_hex(&memory));
        assert_eq!(
            history_of(&session),
            serde_json::json!({"can_undo": true, "can_redo": false, "undo_label": "set a = 2.0",
                               "redo_label": null, "depth": 1}),
            "the undo that failed to persist did not move the cursor"
        );

        // The stale-base check is honest: an apply_text based on the hash
        // the session exposes applies — and only because that text IS on
        // disk. The obstacle is gone (the transient lock released).
        std::fs::remove_dir_all(&sidecar_path).unwrap();
        assert!(
            !session.reload_from_disk("watcher", false).unwrap(),
            "disk and memory agree — nothing for the watcher to reconcile"
        );
        assert_eq!(history_of(&session)["depth"], 1, "no barrier, log intact");
        let (_, hash) = text_and_hash(&session);
        apply(
            &session,
            &hash,
            vec![("p.cic", "# cicada 1\na = 4.0\n")],
            "agent",
        )
        .unwrap();
        session.wait_idle();
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            "# cicada 1\na = 4.0\n"
        );
        assert_eq!(history_of(&session)["depth"], 2);
        // The session is healthy: the undo that was refused works now.
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        assert_eq!(std::fs::read_to_string(&pipeline).unwrap(), before.0);
    }

    #[test]
    fn a_failed_text_write_rolls_the_gesture_back_without_touching_the_sidecar_file() {
        // The other order of failure: the .cic itself cannot be replaced
        // (its temp path is a directory). Nothing lands — not the text,
        // not the sidecar the same gesture would have written.
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let sidecar_path = Sidecar::path_for(&pipeline);
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        let tmp = pipeline.parent().unwrap().join(".p.cic.cicada-tmp");
        std::fs::create_dir(&tmp).unwrap();
        let before = on_disk(&pipeline);
        // place + cell: text AND sidecar change in one gesture.
        session.handle(
            id,
            Some("p".into()),
            ClientMessage::PlaceNode {
                func: "sphere".into(),
                cell: Some([3, 3]),
                connect: None,
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let error = msgs.iter().find(|m| m["type"] == "error").unwrap();
        assert_eq!(error["payload"]["kind"], "persist", "{error}");
        assert_eq!(on_disk(&pipeline), before, "nothing landed");
        assert!(!sidecar_path.exists());
        let (memory, hash) = text_and_hash(&session);
        assert_eq!(memory, before.0);
        assert_eq!(hash, blake3_hex(&memory));
        assert!(
            session.core.lock_inner().sidecar.overrides.is_empty(),
            "the sidecar cell was rolled back in memory"
        );
        assert_eq!(history_of(&session)["depth"], 0);
        std::fs::remove_dir(&tmp).unwrap();
        // And the place works once the obstacle is gone, with the SAME
        // name (the refused place left no trace in the document).
        session.handle(
            id,
            None,
            ClientMessage::PlaceNode {
                func: "sphere".into(),
                cell: Some([3, 3]),
                connect: None,
            },
        );
        session.wait_idle();
        assert!(
            std::fs::read_to_string(&pipeline)
                .unwrap()
                .contains("sphere_1 = sphere("),
        );
        assert_eq!(history_of(&session)["depth"], 1);
    }

    #[test]
    fn a_gesture_that_fails_after_mutating_is_rolled_back_under_the_lock() {
        // `dispatch` (not `handle`) is driven directly: the rollback of a
        // gesture that mutated and then failed — place_node's text edit +
        // sidecar cell landed, then its connect was refused — must happen
        // inside the gesture's own lock hold. `handle`'s old outer rollback
        // ran outside the lock and only when `seq` had not moved; a
        // concurrent writer (HTTP apply_text, the watcher) bumping `seq`
        // between the two lock holds left the half-state in memory for the
        // next op to persist.
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, _rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        let before = on_disk(&pipeline);
        let (memory_before, hash_before) = text_and_hash(&session);
        let error = session
            .dispatch(
                id,
                Some("p".into()),
                ClientMessage::PlaceNode {
                    func: "sphere".into(),
                    cell: Some([3, 3]),
                    connect: Some(crate::protocol::ConnectSpec {
                        from: WireEnd {
                            node: "a".into(),
                            port: "out".into(),
                        },
                        to_port: "nope".into(),
                        lift: false,
                    }),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), "refused", "{error}");
        let inner = session.core.lock_inner();
        assert_eq!(inner.loaded.document.emit(), memory_before);
        assert!(inner.sidecar.overrides.is_empty());
        assert_eq!(hex(&inner.text_hash), hash_before);
        drop(inner);
        assert_eq!(on_disk(&pipeline), before);
        // Undo / redo likewise: a restore whose persist fails puts the
        // cursor back inside the same lock hold.
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        let sidecar_path = Sidecar::path_for(&pipeline);
        std::fs::create_dir(&sidecar_path).unwrap();
        std::fs::write(sidecar_path.join("occupied"), b"x").unwrap();
        let error = session
            .dispatch(id, None, ClientMessage::Undo {})
            .unwrap_err();
        assert_eq!(error.kind(), "persist", "{error}");
        let inner = session.core.lock_inner();
        assert_eq!(inner.oplog.cursor, 1);
        assert_eq!(inner.loaded.document.emit(), "# cicada 1\na = 2.0\n");
        assert_eq!(hex(&inner.text_hash), blake3_hex("# cicada 1\na = 2.0\n"));
        drop(inner);
        assert_eq!(
            std::fs::read_to_string(&pipeline).unwrap(),
            "# cicada 1\na = 2.0\n"
        );
        std::fs::remove_dir_all(&sidecar_path).unwrap();
    }

    #[test]
    fn writes_that_change_nothing_are_answered_but_are_not_undo_steps() {
        // A move to "no cell" on a node that has no override, a set_param
        // to the value already there, an apply_text re-sending the current
        // text: each is acknowledged with a delta (the client asked, the
        // server answers with the authoritative state) but pushes no op —
        // a snapshot op whose before equals its after would be an undo
        // step that visibly does nothing.
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        for (intent, message) in [
            (
                "m",
                ClientMessage::MoveNode {
                    node: "a".into(),
                    cell: None,
                },
            ),
            (
                "s",
                ClientMessage::SetParam {
                    node: "a".into(),
                    port: None,
                    value: "1.0".into(),
                },
            ),
        ] {
            session.handle(id, Some(intent.into()), message);
            session.wait_idle();
            let msgs = texts(&drain(&mut rx));
            let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
            assert_eq!(deltas.len(), 1, "{intent}: acknowledged: {msgs:?}");
            assert_eq!(deltas[0]["payload"]["source"]["intent_id"], intent);
            assert_eq!(
                deltas[0]["payload"]["history"]["depth"], 0,
                "{intent}: not an undo step"
            );
        }
        let (text, hash) = text_and_hash(&session);
        let result = apply(&session, &hash, vec![("p.cic", &text)], "same text").unwrap();
        assert_eq!(result["history"]["depth"], 0);
        assert_eq!(result["text_hash"], hash);
        assert_eq!(history_of(&session)["can_undo"], false);
        assert!(
            session.debug_state(false)["ops"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // A real change right after is op 1 — the no-ops consumed no ids.
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        let ops = session.debug_state(false)["ops"].clone();
        assert_eq!(ops.as_array().unwrap().len(), 1, "{ops}");
        assert_eq!(ops[0]["id"], 1);
    }

    #[test]
    fn a_stale_self_write_hash_never_masks_a_genuine_external_change() {
        // The echo guard is "disk == memory" (text hash, sidecar equality,
        // scripts fingerprint) and nothing else. A remembered
        // "last written" hash would suppress a REAL external change that
        // happens to restore a text this session once wrote: write A,
        // external edit to B (reloaded), external edit back to A — the
        // second reload must happen too, or memory stays B while disk
        // says A.
        let (_dir, config) = project("# cicada 1\na = 1.0\n");
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, _rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        session.handle(
            id,
            None,
            ClientMessage::SetParam {
                node: "a".into(),
                port: None,
                value: "2.0".into(),
            },
        );
        session.wait_idle();
        let text_a = std::fs::read_to_string(&pipeline).unwrap();
        assert_eq!(text_a, "# cicada 1\na = 2.0\n");
        assert!(
            !session.reload_from_disk("echo", false).unwrap(),
            "our own write is an echo"
        );
        std::fs::write(&pipeline, "# cicada 1\na = 5.0\n").unwrap();
        assert!(session.reload_from_disk("to B", false).unwrap());
        session.wait_idle();
        assert_eq!(text_and_hash(&session).0, "# cicada 1\na = 5.0\n");
        std::fs::write(&pipeline, &text_a).unwrap();
        assert!(
            session.reload_from_disk("back to A", false).unwrap(),
            "a genuine external change back to a text we once wrote still reloads"
        );
        session.wait_idle();
        let (memory, hash) = text_and_hash(&session);
        assert_eq!(memory, text_a, "memory follows the disk");
        assert_eq!(hash, blake3_hex(&memory));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one story: disable → ghost with ports → enable as a cache hit → undo/redo → batch
    fn toggle_disable_ghosts_the_node_with_its_ports_and_re_enables_as_a_cache_hit() {
        let source = "# cicada 1\n\
                      size = slider(value=2.0, min=0.5, max=5.0)\n\
                      span = construct_domain(start=0.0, end=size)  # the span\n\
                      block = box(x=span, y=span, z=span)\n";
        let (_dir, config) = project(source);
        let pipeline = config.pipeline.clone();
        let session = Session::open(config).unwrap();
        session.wait_idle();
        let (tx, mut rx) = unbounded_channel();
        let (id, _) = session.connect(tx);
        drain(&mut rx);
        let before = session.debug_state(false);
        assert_eq!(before["statuses"]["block"]["state"], "done");

        // ---- disable `span`: ONE op labelled `disable span`; the text gains
        // exactly the `#off ` prefix; the ghost keeps its ports, its literal
        // and its incoming wire; downstream is red for the precise reason.
        session.handle(
            id,
            Some("d1".into()),
            ClientMessage::ToggleDisable {
                node: "span".into(),
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
        assert_eq!(deltas.len(), 1, "{msgs:?}");
        let delta = &deltas[0]["payload"];
        assert_eq!(delta["source"]["label"], "disable span");
        assert_eq!(delta["source"]["intent_id"], "d1");
        let disabled_text = "# cicada 1\n\
                             size = slider(value=2.0, min=0.5, max=5.0)\n\
                             #off span = construct_domain(start=0.0, end=size)  # the span\n\
                             block = box(x=span, y=span, z=span)\n";
        assert_eq!(delta["text"], disabled_text);
        assert_eq!(on_disk(&pipeline).0, disabled_text, "persisted at once");
        assert_eq!(delta["history"]["depth"], 1);
        assert_eq!(delta["history"]["undo_label"], "disable span");
        let dirty: Vec<&str> = delta["dirty"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        assert_eq!(
            dirty,
            ["span", "block"],
            "the node and its dependents flash"
        );

        let graph = &delta["graph"];
        let span = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == "span")
            .expect("the ghost is still a node");
        assert_eq!(span["kind"], "disabled");
        assert_eq!(span["func"], "construct_domain");
        assert_eq!(
            span["title"], "Construct Domain",
            "the spec's title, not `disabled`"
        );
        assert_eq!(span["excluded"]["status"], "red");
        assert_eq!(span["excluded"]["reason"], "disabled (`#off`)");
        let inputs: Vec<&str> = span["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect();
        assert_eq!(inputs, ["start", "end"], "ports intact");
        assert_eq!(
            span["inputs"][0]["literal"], "0.0",
            "the literal is still shown"
        );
        assert_eq!(span["inputs"][1]["wired"]["node"], "size");
        assert_eq!(span["outputs"][0]["name"], "out");
        assert_eq!(
            span["text"], "#off span = construct_domain(start=0.0, end=size)  # the span",
            "the ghost shows its raw line, prefix and trailing comment included"
        );
        let wires: Vec<&str> = graph["wires"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["id"].as_str().unwrap())
            .collect();
        assert!(
            wires.contains(&"size.out->span.end"),
            "the ghost's incoming wire is drawn: {wires:?}"
        );
        assert!(
            wires.contains(&"span.out->block.x"),
            "the downstream wire is drawn (red): {wires:?}"
        );
        let red = graph["wires"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == "span.out->block.x")
            .unwrap();
        assert_eq!(red["red"], true);
        assert!(
            red["reason"].as_str().unwrap().contains("disabled"),
            "{}",
            red["reason"]
        );
        let block = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == "block")
            .unwrap();
        assert_eq!(block["kind"], "call");
        assert!(
            block["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["message"]
                    .as_str()
                    .unwrap()
                    .contains("`span` is disabled")),
            "downstream names the disabled node, never unknown-name: {}",
            block["diagnostics"]
        );
        let state = session.debug_state(false);
        assert_eq!(state["statuses"]["block"]["state"], "red");
        assert_eq!(state["statuses"]["span"]["state"], "red");
        assert_eq!(
            state["statuses"]["size"]["state"], "cached",
            "upstream untouched"
        );

        // ---- the ghost refuses in-place edits BY NAME (not "no binding").
        let error = session
            .dispatch(
                id,
                None,
                ClientMessage::SetParam {
                    node: "span".into(),
                    port: Some("start".into()),
                    value: "1.0".into(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), "writer");
        assert!(error.to_string().contains("`span` is disabled"), "{error}");
        // …but can be moved (sidecar) and deleted — it is the user's line.
        session.handle(
            id,
            None,
            ClientMessage::MoveNode {
                node: "span".into(),
                cell: Some([9, 9]),
            },
        );
        session.wait_idle();
        drain(&mut rx);
        assert_eq!(history_of(&session)["undo_label"], "move span");

        // ---- enable again: `enable span`, the text is byte-identical to the
        // start, and nothing recomputes — every output is a memo hit.
        session.handle(
            id,
            Some("e1".into()),
            ClientMessage::ToggleDisable {
                node: "span".into(),
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let deltas: Vec<_> = msgs.iter().filter(|m| m["type"] == "delta").collect();
        assert_eq!(deltas.len(), 1, "{msgs:?}");
        assert_eq!(deltas[0]["payload"]["source"]["label"], "enable span");
        assert_eq!(deltas[0]["payload"]["text"], source);
        assert_eq!(on_disk(&pipeline).0, source);
        let state = session.debug_state(false);
        assert_eq!(
            state["statuses"]["span"]["state"], "cached",
            "{}",
            state["statuses"]
        );
        assert_eq!(
            state["statuses"]["block"]["state"], "cached",
            "{}",
            state["statuses"]
        );
        assert_eq!(history_of(&session)["depth"], 3);

        // ---- undo the enable: the ghost is back (with its moved cell);
        // redo: live again. Snapshots, so both are restores.
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert_eq!(delta["payload"]["source"]["label"], "undo: enable span");
        assert_eq!(delta["payload"]["text"], disabled_text);
        let span = delta["payload"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["name"] == "span")
            .unwrap();
        assert_eq!(span["kind"], "disabled");
        assert_eq!(span["cell"], serde_json::json!([9, 9]));
        session.handle(id, None, ClientMessage::Redo {});
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert_eq!(delta["payload"]["source"]["label"], "redo: enable span");
        assert_eq!(delta["payload"]["text"], source);

        // ---- a multi-select `D` is one batch: two toggles, one op, one
        // undo; a mixed selection flips each its own way.
        session.handle(
            id,
            None,
            ClientMessage::ToggleDisable {
                node: "size".into(),
            },
        );
        session.wait_idle();
        drain(&mut rx);
        session.handle(
            id,
            Some("b".into()),
            ClientMessage::Batch {
                ops: vec![
                    ClientMessage::ToggleDisable {
                        node: "size".into(),
                    },
                    ClientMessage::ToggleDisable {
                        node: "block".into(),
                    },
                ],
                label: "toggle 2 nodes".into(),
            },
        );
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert_eq!(delta["payload"]["source"]["label"], "toggle 2 nodes");
        assert_eq!(
            delta["payload"]["text"],
            "# cicada 1\n\
             size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)  # the span\n\
             #off block = box(x=span, y=span, z=span)\n"
        );
        let depth = history_of(&session)["depth"].as_u64().unwrap();
        session.handle(id, None, ClientMessage::Undo {});
        session.wait_idle();
        let msgs = texts(&drain(&mut rx));
        let delta = msgs.iter().find(|m| m["type"] == "delta").unwrap();
        assert_eq!(delta["payload"]["source"]["label"], "undo: toggle 2 nodes");
        assert_eq!(
            delta["payload"]["text"],
            "# cicada 1\n\
             #off size = slider(value=2.0, min=0.5, max=5.0)\n\
             span = construct_domain(start=0.0, end=size)  # the span\n\
             block = box(x=span, y=span, z=span)\n"
        );
        assert_eq!(history_of(&session)["depth"], depth - 1);

        // ---- unknown names are loud, and a batch that hits one rolls back whole.
        let error = session
            .dispatch(
                id,
                None,
                ClientMessage::ToggleDisable {
                    node: "nope".into(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), "writer");
        assert!(
            error.to_string().contains("no binding named `nope`"),
            "{error}"
        );
    }
}
