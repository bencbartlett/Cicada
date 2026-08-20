//! The executor (docs/12 §Execution, §Cancellation): wavefront over the
//! DAG on a rayon pool sized `cores − 2`, per-element fan-out for `each()`
//! nodes in cost-sized chunks, one cancellation token per generation
//! (owned by the `solve` call, handed to every node invocation as its
//! [`NodeCtx`] — see [`crate::cancel`]) checked between nodes and between
//! chunks, node panics caught and turned into red nodes, completed work
//! written through to the store — which is why cancellation and
//! supersession are nearly free. Two flags gate the memo: `effectful`
//! nodes never consult it (their work IS the side effect) and `volatile`
//! nodes never consult it either (their value is fresh by definition —
//! docs/12 §Volatile nodes); both run every time their cone is solved. One honest
//! qualifier: element-LEVEL persistence needs calibrated cost stats
//! (docs/12 adaptive granularity), so a cold node's very first fan
//! persists at node granularity only — a cancelled cold fan still records
//! its partial cost sample, so the next generation calibrates and
//! element caching turns on.
//!
//! Determinism: element results land in pre-sized, index-addressed slots;
//! execution order can never reorder output (docs/12 §Determinism rules).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use cicada_core::hash::ValueHash;
use cicada_core::value::{HashedValue, List, ValueData};

use crate::cancel::{CancelToken, NodeCtx};
use crate::clock::Clock;
use crate::cost::{self, CostSample};
use crate::graph::{Input, NodeDecl, NodeId, SolveGraph};
use crate::key::{KeyInputs, NodeKey, node_key};
use crate::store::{DiskStore, MemoEntry, StoreError};

/// Scheduler tuning. Defaults follow docs/12; every threshold is explicit
/// so tests pin behavior instead of guessing.
#[derive(Debug, Clone, Copy)]
pub struct SchedulerConfig {
    /// Worker threads; `0` = `cores − 2`, minimum 1 (docs/12: UI and OS
    /// breathe).
    pub threads: usize,
    /// Target work per element chunk, nanoseconds (docs/12: ~10–50 ms;
    /// default 25 ms).
    pub chunk_target_nanos: u64,
    /// Cold-start per-element estimate when no samples exist (rough by
    /// definition — the first solve calibrates).
    pub cold_element_nanos: u64,
    /// Per-element memoization only when the measured per-element cost
    /// exceeds this (docs/12 §Adaptive cache granularity: booleans yes,
    /// `x * 2` no).
    pub element_cache_min_nanos: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            threads: 0,
            chunk_target_nanos: 25_000_000,
            cold_element_nanos: 1_000_000,
            element_cache_min_nanos: 100_000,
        }
    }
}

/// Execution events, delivered synchronously from worker threads. The
/// stage-5 progress/ETA display consumes these; tests record them as the
/// oracle for cache hits, dirty cones, and cancellation latency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// A node began resolving (key building — not necessarily computing).
    NodeStarted {
        /// Node display name.
        node: &'a str,
    },
    /// The memo table answered — nothing ran.
    NodeCacheHit {
        /// Node display name.
        node: &'a str,
    },
    /// One element answered from the element-level memo.
    ElementCacheHit {
        /// Node display name.
        node: &'a str,
        /// Element index.
        index: usize,
    },
    /// One chunk of an `each()` fan-out finished executing.
    ChunkExecuted {
        /// Node display name.
        node: &'a str,
        /// First element index of the chunk.
        start: usize,
        /// Elements in the chunk.
        len: usize,
        /// Measured work nanoseconds (clock-injected — virtual in tests).
        nanos: u64,
    },
    /// A node finished computing.
    NodeComputed {
        /// Node display name.
        node: &'a str,
        /// Elements processed (1 for scalar nodes).
        elements: u64,
        /// Total measured work nanoseconds.
        nanos: u64,
    },
    /// A node failed (red).
    NodeFailed {
        /// Node display name.
        node: &'a str,
    },
    /// A node did not run because an upstream failed.
    NodeBlocked {
        /// Node display name.
        node: &'a str,
        /// The failed/blocked upstream's name.
        upstream: &'a str,
    },
    /// A node did not (fully) run because the generation was cancelled.
    NodeCancelled {
        /// Node display name.
        node: &'a str,
    },
}

/// Receives [`Event`]s. Implementations must be cheap and thread-safe.
pub trait Observer: Send + Sync {
    /// One event.
    fn on_event(&self, event: &Event<'_>);
}

/// `CICADA_TRACE=1` prints per-node phase timings (key/memo, hydrate,
/// run, commit) to stderr — the measurement loop's profiler until the real
/// one lands (docs/12 §Progress; stage 6 used it to find a 250 ms hydrate
/// hiding behind a 15 ms Python node). Read once.
fn trace_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("CICADA_TRACE").is_some_and(|v| !v.is_empty()))
}

/// Ignores everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopObserver;

impl Observer for NoopObserver {
    fn on_event(&self, _event: &Event<'_>) {}
}

/// A red node: name, message, and (for element fan-outs) the offending
/// element IDs (docs/12 §Element failures — loud refusal, wall lesson 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFailure {
    /// The node.
    pub node: String,
    /// What went wrong.
    pub message: String,
    /// Offending element indices (empty for whole-node failures).
    pub element_ids: Vec<usize>,
}

/// Per-node result of one generation.
#[derive(Debug, Clone)]
pub enum NodeOutcome {
    /// Outside the requested cone — untouched by construction.
    Skipped,
    /// Answered by the memo table.
    CacheHit {
        /// Output hashes, port order.
        outputs: Vec<ValueHash>,
        /// What the computation cost when it last ran, when the entry
        /// recorded it (node-level entries since v0.1): the cost model's
        /// evidence for a node that computed nothing this generation.
        cost: Option<CostSample>,
    },
    /// Computed this generation.
    Computed {
        /// Output hashes, port order.
        outputs: Vec<ValueHash>,
        /// Elements processed (1 for scalar nodes).
        elements: u64,
        /// Measured work nanoseconds.
        nanos: u64,
    },
    /// Red.
    Failed(NodeFailure),
    /// An upstream is red or blocked; this node did not run.
    Blocked {
        /// The upstream's name.
        upstream: String,
    },
    /// The generation was cancelled before/while this node ran.
    Cancelled,
}

impl NodeOutcome {
    /// Output hashes when the node has them (cache hit or computed).
    #[must_use]
    pub fn output_hashes(&self) -> Option<&[ValueHash]> {
        match self {
            Self::CacheHit { outputs, .. } | Self::Computed { outputs, .. } => Some(outputs),
            _ => None,
        }
    }
}

/// One generation's report.
#[derive(Debug, Clone)]
pub struct SolveReport {
    /// Which generation.
    pub generation: u64,
    /// True when the generation's token was cancelled.
    pub cancelled: bool,
    /// Per-node outcomes, indexed by `NodeId`.
    pub outcomes: Vec<NodeOutcome>,
}

impl SolveReport {
    /// One node's outcome.
    #[must_use]
    pub fn outcome(&self, id: NodeId) -> &NodeOutcome {
        &self.outcomes[id.0]
    }

    /// Every failure, in node order.
    #[must_use]
    pub fn failures(&self) -> Vec<&NodeFailure> {
        self.outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                NodeOutcome::Failed(failure) => Some(failure),
                _ => None,
            })
            .collect()
    }
}

/// Engine-level solve failures (node failures are outcomes, not errors).
#[derive(Debug, thiserror::Error)]
pub enum SolveError {
    /// The store refused — the generation aborts; completed work up to the
    /// failure is already persisted.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The rayon pool could not be built.
    #[error("thread pool: {message}")]
    Pool {
        /// Builder error text.
        message: String,
    },
    /// A panic escaped the engine itself (an observer, an executor
    /// invariant) during a preview generation — caught and surfaced so the
    /// session never wedges silently.
    #[error("engine panic during solve: {message}")]
    EnginePanic {
        /// The panic payload's text.
        message: String,
    },
}

/// The scheduler: a thread pool + store + clock, solving graphs.
pub struct Scheduler {
    pool: rayon::ThreadPool,
    store: Arc<DiskStore>,
    clock: Arc<dyn Clock>,
    config: SchedulerConfig,
    threads: usize,
}

impl Scheduler {
    /// Build with the given store, clock, and tuning.
    ///
    /// # Errors
    ///
    /// [`SolveError::Pool`] when the thread pool cannot start.
    pub fn new(
        store: Arc<DiskStore>,
        clock: Arc<dyn Clock>,
        config: SchedulerConfig,
    ) -> Result<Self, SolveError> {
        let threads = if config.threads == 0 {
            std::thread::available_parallelism()
                .map_or(2, std::num::NonZeroUsize::get)
                .saturating_sub(2)
                .max(1)
        } else {
            config.threads
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("cicada-solve-{index}"))
            .build()
            .map_err(|error| SolveError::Pool {
                message: error.to_string(),
            })?;
        Ok(Self {
            pool,
            store,
            clock,
            config,
            threads,
        })
    }

    /// The store (callers load output values through it).
    #[must_use]
    pub fn store(&self) -> &Arc<DiskStore> {
        &self.store
    }

    /// Worker thread count.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Solve one generation: pull-compute `targets` and their upstream
    /// cone. Nodes outside the cone are [`NodeOutcome::Skipped`]; dirtiness
    /// inside it is exact by content addressing — unchanged keys hit the
    /// memo and execute nothing.
    ///
    /// # Errors
    ///
    /// [`SolveError`] on engine-level failures (store I/O). Node failures
    /// are reported per node, never as an `Err`.
    ///
    /// # Panics
    ///
    /// On broken internal wavefront invariants (a needed node left
    /// unresolved, a node scheduled twice) — bugs, surfaced loudly.
    pub fn solve(
        &self,
        graph: &SolveGraph,
        targets: &[NodeId],
        generation: u64,
        token: &CancelToken,
        observer: &dyn Observer,
    ) -> Result<SolveReport, SolveError> {
        let needed = graph.ancestors(targets);
        let pending: Vec<AtomicUsize> = (0..graph.len())
            .map(|index| AtomicUsize::new(graph.upstream(NodeId(index)).len()))
            .collect();
        let ctx = Ctx {
            scheduler: self,
            graph,
            needed,
            pending,
            outcomes: (0..graph.len()).map(|_| OnceLock::new()).collect(),
            values: (0..graph.len()).map(|_| Mutex::new(Vec::new())).collect(),
            keys: (0..graph.len()).map(|_| OnceLock::new()).collect(),
            token,
            observer,
            fatal: Mutex::new(None),
        };

        let ctx_ref = &ctx;
        self.pool.scope(|scope| {
            for index in 0..graph.len() {
                let id = NodeId(index);
                if ctx_ref.needed[index] && graph.upstream(id).is_empty() {
                    scope.spawn(move |scope| ctx_ref.run_node(scope, id));
                }
            }
        });

        if let Some(error) = ctx
            .fatal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }

        let needed = &ctx.needed;
        let outcomes = ctx
            .outcomes
            .into_iter()
            .enumerate()
            .map(|(index, slot)| {
                slot.into_inner().unwrap_or_else(|| {
                    // Unset ⇒ not needed. A needed node left unresolved is
                    // a broken wavefront — loud in every build, never a
                    // silent Skipped.
                    assert!(!needed[index], "needed node {index} left unresolved");
                    NodeOutcome::Skipped
                })
            })
            .collect();
        Ok(SolveReport {
            generation,
            cancelled: token.is_cancelled(),
            outcomes,
        })
    }
}

/// One element's result in a fan-out: outputs, or (index, message).
type ElementResult = Result<Vec<Arc<HashedValue>>, (usize, String)>;

/// The fanned input lists of one node, by input slot, plus the zip length.
type FannedInputs<'v> = (Vec<(usize, &'v List)>, usize);

/// How one node's computation ended, before it becomes an outcome.
enum Computed {
    Done {
        outputs: Vec<ValueHash>,
        /// Elements processed (element-cache hits included) — display.
        elements: u64,
        /// Elements actually EXECUTED — the cost-sample denominator.
        /// Counting cache hits here would dilute per-element estimates
        /// toward zero with warm use, silently growing chunks past the
        /// cancellation budget.
        computed: u64,
        nanos: u64,
    },
    Failed(NodeFailure),
    Cancelled,
}

/// Shared solve state, borrowed by every worker task.
struct Ctx<'a> {
    scheduler: &'a Scheduler,
    graph: &'a SolveGraph,
    needed: Vec<bool>,
    pending: Vec<AtomicUsize>,
    outcomes: Vec<OnceLock<NodeOutcome>>,
    /// Hydrated output values per node (computed this generation, or
    /// lazily loaded from the store on first downstream need — a warm
    /// solve whose values nobody asks for loads no blobs at all).
    /// Hydrated output values per node, PER PORT (`None` = not loaded
    /// yet): a consumer of one port of a 17-output node must not pay for
    /// loading the other sixteen (stage-6 measurement: hydrating the wall
    /// layout node cost 250 ms per slider tick for one 1,200-point port).
    values: Vec<Mutex<Vec<Option<Arc<HashedValue>>>>>,
    /// Each node's `NodeKey`, recorded at resolve time — hydrate failures
    /// invalidate the promising memo entry through it (self-heal).
    keys: Vec<OnceLock<NodeKey>>,
    token: &'a CancelToken,
    observer: &'a dyn Observer,
    fatal: Mutex<Option<SolveError>>,
}

impl Ctx<'_> {
    /// Record a fatal engine error (first one wins) and cancel the
    /// generation so workers drain quickly.
    fn set_fatal(&self, error: StoreError) {
        let mut fatal = self
            .fatal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if fatal.is_none() {
            *fatal = Some(SolveError::Store(error));
        }
        self.token.cancel();
    }

    fn run_node<'s>(&'s self, scope: &rayon::Scope<'s>, id: NodeId) {
        let outcome = self.resolve_node(id);
        match &outcome {
            NodeOutcome::Failed(_) => self.observer.on_event(&Event::NodeFailed {
                node: &self.graph.node(id).name,
            }),
            NodeOutcome::Cancelled => self.observer.on_event(&Event::NodeCancelled {
                node: &self.graph.node(id).name,
            }),
            _ => {}
        }
        assert!(
            self.outcomes[id.0].set(outcome).is_ok(),
            "node scheduled twice — wavefront invariant broken"
        );
        for &dependent in self.graph.dependents(id) {
            if self.needed[dependent.0]
                && self.pending[dependent.0].fetch_sub(1, Ordering::AcqRel) == 1
            {
                scope.spawn(move |scope| self.run_node(scope, dependent));
            }
        }
    }

    /// The full outcome of one node: upstream gates, key, memo, compute.
    fn resolve_node(&self, id: NodeId) -> NodeOutcome {
        if self.token.is_cancelled() {
            return NodeOutcome::Cancelled;
        }
        let decl = self.graph.node(id);
        self.observer
            .on_event(&Event::NodeStarted { node: &decl.name });

        // Upstream gates: a red/blocked upstream blocks this node; a
        // cancelled upstream cancels it.
        for &up in self.graph.upstream(id) {
            match self.outcomes[up.0].get() {
                Some(NodeOutcome::Failed(_) | NodeOutcome::Blocked { .. }) => {
                    let upstream = &self.graph.node(up).name;
                    self.observer.on_event(&Event::NodeBlocked {
                        node: &decl.name,
                        upstream,
                    });
                    return NodeOutcome::Blocked {
                        upstream: upstream.clone(),
                    };
                }
                Some(NodeOutcome::Cancelled) => return NodeOutcome::Cancelled,
                Some(_) => {}
                None => {
                    // The pending counters guarantee upstreams resolve
                    // first; an unset upstream is a broken wavefront —
                    // loud, never a mislabeled Blocked.
                    unreachable!(
                        "`{}` scheduled before upstream `{}` resolved",
                        decl.name,
                        self.graph.node(up).name
                    );
                }
            }
        }

        let trace = trace_enabled();
        let phase_started = std::time::Instant::now();
        // Input hashes → key. Hash-only: no value is loaded for a memo hit.
        let input_hashes: Vec<Option<ValueHash>> = decl
            .inputs
            .iter()
            .map(|input| match input {
                Input::Value(value) => Some(value.hash()),
                Input::Absent => None,
                Input::Port { node, output } => self.outcomes[node.0]
                    .get()
                    .and_then(NodeOutcome::output_hashes)
                    .map(|outputs| outputs[*output]),
            })
            .collect();
        let key = node_key(&KeyInputs {
            op: &decl.op,
            version: decl.version,
            body_hash: decl.body_hash.as_ref(),
            tolerance: decl.tolerance.as_ref(),
            inputs: &input_hashes,
            fan: &decl.fan,
        });
        let _ = self.keys[id.0].set(key);
        // Effectful nodes (exporters) never consult the memo: their WORK is
        // the side effect, and a hit would silently skip it (doc 10 §7).
        // Volatile nodes never consult it either: their value is fresh by
        // definition — a hit would serve a stale clock (docs/12 §Volatile
        // nodes). Both gates are per node; downstream nodes key on the
        // fresh output hash like any other input.
        if !decl.effectful
            && !decl.volatile
            && let Some(entry) = self.scheduler.store.memo(&key)
        {
            if entry.outputs.len() == decl.output_count {
                self.observer
                    .on_event(&Event::NodeCacheHit { node: &decl.name });
                if trace {
                    eprintln!(
                        "trace: {} cache hit in {:.3} ms",
                        decl.name,
                        phase_started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                return NodeOutcome::CacheHit {
                    outputs: entry.outputs,
                    cost: entry.cost,
                };
            }
            // A record whose arity disagrees with the node is corrupt,
            // stale, or foreign. Trusting it would index out of bounds
            // downstream; the cache never owes correctness (docs/12), so
            // tombstone it and recompute.
            if let Err(error) = self.scheduler.store.invalidate_memo(key) {
                self.set_fatal(error);
                return NodeOutcome::Cancelled;
            }
        }

        self.compute_and_record(id, decl, key, trace, phase_started)
    }

    /// The memo-miss path: hydrate inputs, compute, persist, memoize —
    /// with the optional phase trace.
    fn compute_and_record(
        &self,
        id: NodeId,
        decl: &NodeDecl,
        key: NodeKey,
        trace: bool,
        phase_started: std::time::Instant,
    ) -> NodeOutcome {
        let key_ms = phase_started.elapsed().as_secs_f64() * 1000.0;
        let hydrate_started = std::time::Instant::now();
        let input_values = match self.gather_input_values(decl) {
            Ok(values) => values,
            Err(error) => {
                self.set_fatal(error);
                return NodeOutcome::Cancelled;
            }
        };
        let hydrate_ms = hydrate_started.elapsed().as_secs_f64() * 1000.0;
        let run_started = std::time::Instant::now();
        let computed = if decl.fan.iter().any(|&depth| depth > 0) {
            self.compute_fanned(id, decl, &input_values)
        } else {
            self.compute_scalar(id, decl, &input_values)
        };
        let run_ms = run_started.elapsed().as_secs_f64() * 1000.0;
        let commit_started = std::time::Instant::now();
        let outcome = match computed {
            Computed::Cancelled => NodeOutcome::Cancelled,
            // A failure under a cancelled token is the cancellation
            // surfacing — a killed worker's "cancelled" error, a long loop
            // bailing at its safe point — not a red node: the generation
            // was cancelled, and the next one re-runs it either way
            // (failures are never memoized).
            Computed::Failed(_) if self.token.is_cancelled() => NodeOutcome::Cancelled,
            Computed::Failed(failure) => NodeOutcome::Failed(failure),
            Computed::Done {
                outputs,
                elements,
                computed,
                nanos,
            } => self.record_done(decl, key, outputs, elements, computed, nanos),
        };
        if trace {
            eprintln!(
                "trace: {} key {key_ms:.3} ms, hydrate {hydrate_ms:.3} ms, run+persist {run_ms:.3} ms, memo {:.3} ms",
                decl.name,
                commit_started.elapsed().as_secs_f64() * 1000.0
            );
        }
        outcome
    }

    /// Memo + cost sample + event for a completed compute.
    fn record_done(
        &self,
        decl: &NodeDecl,
        key: NodeKey,
        outputs: Vec<ValueHash>,
        elements: u64,
        computed: u64,
        nanos: u64,
    ) -> NodeOutcome {
        // Effectful and volatile nodes are never memoized (see the memo-read
        // gate); the cost sample below still records — the estimator can
        // know an export's (or a clock's cone's) cost without the cache
        // ever lying about having run it. The entry carries the cost too,
        // but only when the computation executed EVERY element — then
        // `nanos` is unambiguously what computing this key from scratch
        // cost and `elements` its size. A fan partly (or wholly) served
        // from the element cache measured only the elements that ran; an
        // entry saying "1 ms · 1,200 elements" — or "0 ns" — for it would
        // be the cache lying about what the work costs, so it records no
        // cost (the op-level sample below still learns from what ran).
        if !decl.effectful && !decl.volatile {
            let recorded = if computed == elements {
                self.scheduler.store.record_memo_with_cost(
                    key,
                    &outputs,
                    CostSample { elements, nanos },
                )
            } else {
                self.scheduler.store.record_memo(key, &outputs)
            };
            if let Err(error) = recorded {
                self.set_fatal(error);
                return NodeOutcome::Cancelled;
            }
        }
        // Sample only what actually executed; a fully-warm fan teaches
        // nothing and would dilute the estimate.
        if computed > 0
            && let Err(error) = self
                .scheduler
                .store
                .record_sample(&decl.op, computed, nanos)
        {
            self.set_fatal(error);
            return NodeOutcome::Cancelled;
        }
        self.observer.on_event(&Event::NodeComputed {
            node: &decl.name,
            elements,
            nanos,
        });
        NodeOutcome::Computed {
            outputs,
            elements,
            nanos,
        }
    }

    /// Per-port input values (None = absent/default). Upstream values come
    /// from this generation's compute or lazily from the store.
    fn gather_input_values(
        &self,
        decl: &NodeDecl,
    ) -> Result<Vec<Option<Arc<HashedValue>>>, StoreError> {
        decl.inputs
            .iter()
            .map(|input| match input {
                Input::Value(value) => Ok(Some(Arc::clone(value))),
                Input::Absent => Ok(None),
                Input::Port { node, output } => self.hydrate(*node, *output).map(Some),
            })
            .collect()
    }

    /// One output value of a node, loading it from the store on first need
    /// (only that port — the others stay hashes until someone asks).
    fn hydrate(&self, id: NodeId, output: usize) -> Result<Arc<HashedValue>, StoreError> {
        let mut slots = self.values[id.0]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(Some(value)) = slots.get(output) {
            return Ok(Arc::clone(value));
        }
        let Some(hashes) = self.outcomes[id.0]
            .get()
            .and_then(NodeOutcome::output_hashes)
            .map(<[ValueHash]>::to_vec)
        else {
            unreachable!("hydrate on a node without outputs — wavefront invariant broken");
        };
        if slots.len() < hashes.len() {
            slots.resize(hashes.len(), None);
        }
        let Some(hash) = hashes.get(output) else {
            unreachable!(
                "hydrate of output {output} on a node with {} outputs",
                hashes.len()
            );
        };
        match self.scheduler.store.load_value(hash) {
            Ok(value) => {
                slots[output] = Some(Arc::clone(&value));
                Ok(value)
            }
            Err(error) => {
                // The memo entry PROMISED this value was loadable and
                // broke the promise (quarantined blob). Tombstone the
                // entry so the NEXT solve recomputes instead of
                // re-hitting a dead record forever; this generation
                // still fails loudly with the load error.
                if let Some(key) = self.keys[id.0].get()
                    && let Err(invalidate_error) = self.scheduler.store.invalidate_memo(*key)
                {
                    return Err(invalidate_error);
                }
                Err(error)
            }
        }
    }

    /// Store outputs and stash them for downstream reuse.
    fn commit_outputs(
        &self,
        id: NodeId,
        values: Vec<Arc<HashedValue>>,
    ) -> Result<Vec<ValueHash>, StoreError> {
        for value in &values {
            let started = std::time::Instant::now();
            self.scheduler.store.store_value(value)?;
            if trace_enabled() {
                eprintln!(
                    "trace:   store {} ({} bytes-ish) in {:.3} ms",
                    value.data().kind_name(),
                    match value.data() {
                        ValueData::List(list) => list.slots.len(),
                        _ => 1,
                    },
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        let hashes = values.iter().map(|value| value.hash()).collect();
        *self.values[id.0]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            values.into_iter().map(Some).collect();
        Ok(hashes)
    }

    /// One plain (non-fanned) node call.
    fn compute_scalar(
        &self,
        id: NodeId,
        decl: &NodeDecl,
        inputs: &[Option<Arc<HashedValue>>],
    ) -> Computed {
        let clock = &self.scheduler.clock;
        let ctx = NodeCtx { cancel: self.token };
        let start = clock.now_nanos();
        let result = catch_unwind(AssertUnwindSafe(|| (decl.run)(&ctx, inputs)));
        let nanos = clock.now_nanos().saturating_sub(start);
        let outputs = match result {
            Err(payload) => {
                return Computed::Failed(NodeFailure {
                    node: decl.name.clone(),
                    message: panic_message(payload.as_ref()),
                    element_ids: Vec::new(),
                });
            }
            Ok(Err(error)) => {
                return Computed::Failed(NodeFailure {
                    node: decl.name.clone(),
                    message: error.message,
                    element_ids: Vec::new(),
                });
            }
            Ok(Ok(outputs)) => outputs,
        };
        if outputs.len() != decl.output_count {
            return Computed::Failed(NodeFailure {
                node: decl.name.clone(),
                message: format!(
                    "node returned {} outputs; its spec declares {}",
                    outputs.len(),
                    decl.output_count
                ),
                element_ids: Vec::new(),
            });
        }
        match self.commit_outputs(id, outputs) {
            Err(error) => {
                self.set_fatal(error);
                Computed::Cancelled
            }
            Ok(hashes) => Computed::Done {
                outputs: hashes,
                elements: 1,
                computed: 1,
                nanos,
            },
        }
    }

    /// An `each()` fan-out: strict zip, chunked element execution, results
    /// into pre-sized slots, failures red-with-IDs.
    fn compute_fanned(
        &self,
        id: NodeId,
        decl: &NodeDecl,
        inputs: &[Option<Arc<HashedValue>>],
    ) -> Computed {
        let (fanned, n) = match fanned_shape(decl, inputs) {
            Ok(shape) => shape,
            Err(failure) => return Computed::Failed(failure),
        };
        let axis = fanned.first().and_then(|(_, list)| list.axis.clone());
        if n == 0 {
            return self.finish_fanned(id, decl, axis.as_ref(), &[], 0, 0);
        }

        // Chunk size from measured cost (docs/12: ~10–50 ms of work); the
        // same estimate decides element-level memoization (docs/12
        // §Adaptive cache granularity — measured, not guessed).
        let per_element = self
            .scheduler
            .store
            .stats(&decl.op)
            .and_then(|stats| stats.per_element_nanos());
        let chunk = cost::chunk_elements(
            per_element,
            self.scheduler.config.cold_element_nanos,
            self.scheduler.config.chunk_target_nanos,
            n,
            self.scheduler.threads,
        );
        // Effectful nodes never memoize at ANY granularity: an each()-
        // lifted exporter served per-element from cache would silently
        // skip side effects for the warm elements (same rule as the
        // node-level gate — the cache must never lie about work done).
        // Volatile nodes likewise: a volatile node inside a fan-out
        // recomputes PER ELEMENT, every generation.
        let element_cache = !decl.effectful
            && !decl.volatile
            && per_element
                .is_some_and(|nanos| nanos >= self.scheduler.config.element_cache_min_nanos);

        let (results, nanos, computed) =
            self.run_chunks(decl, inputs, &fanned, n, chunk, element_cache);

        if self
            .fatal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
        {
            return Computed::Cancelled;
        }
        if results.iter().any(Option::is_none) {
            // Cancelled mid-fan: completed elements are already in the
            // store when element-cached; the node yields no output this
            // generation. The partial work is still legitimate
            // CALIBRATION data — without this sample, a preview stream
            // superseding a cold expensive map would never accrue stats,
            // element caching would never enable, and every generation
            // would restart from zero, forever.
            if computed > 0
                && let Err(error) = self
                    .scheduler
                    .store
                    .record_sample(&decl.op, computed, nanos)
            {
                self.set_fatal(error);
            }
            return Computed::Cancelled;
        }

        // Failures: red with ALL offending element IDs (docs/12).
        let mut failures: Vec<(usize, String)> = Vec::new();
        let mut element_outputs: Vec<Vec<Arc<HashedValue>>> = Vec::with_capacity(n);
        for result in results.into_iter().flatten() {
            match result {
                Ok(outputs) => element_outputs.push(outputs),
                Err(failure) => failures.push(failure),
            }
        }
        if !failures.is_empty() {
            failures.sort_by_key(|(index, _)| *index);
            let (first_id, first_message) = &failures[0];
            let message = if failures.len() == 1 {
                format!("element {first_id}: {first_message}")
            } else {
                format!(
                    "element {first_id}: {first_message} (and {} more elements)",
                    failures.len() - 1
                )
            };
            return Computed::Failed(NodeFailure {
                node: decl.name.clone(),
                message,
                element_ids: failures.into_iter().map(|(index, _)| index).collect(),
            });
        }

        self.finish_fanned(id, decl, axis.as_ref(), &element_outputs, nanos, computed)
    }

    /// Execute the chunks of a fan-out on the pool; results land in
    /// pre-sized, index-addressed slots (docs/12 §Determinism rules).
    /// Returns (results, work nanos, elements actually EXECUTED — cache
    /// hits excluded, they are not cost evidence).
    fn run_chunks(
        &self,
        decl: &NodeDecl,
        inputs: &[Option<Arc<HashedValue>>],
        fanned: &[(usize, &List)],
        n: usize,
        chunk: usize,
        element_cache: bool,
    ) -> (Vec<Option<ElementResult>>, u64, u64) {
        let mut results: Vec<Option<ElementResult>> =
            std::iter::repeat_with(|| None).take(n).collect();
        let total_nanos = AtomicU64::new(0);
        let executed = AtomicU64::new(0);
        rayon::scope(|scope| {
            for (chunk_index, chunk_slots) in results.chunks_mut(chunk).enumerate() {
                let start = chunk_index * chunk;
                let total_nanos = &total_nanos;
                let executed = &executed;
                scope.spawn(move |_| {
                    // The between-chunks cancellation point (docs/12).
                    if self.token.is_cancelled() {
                        return;
                    }
                    let clock = &self.scheduler.clock;
                    let begin = clock.now_nanos();
                    let mut done = 0;
                    for (offset, slot) in chunk_slots.iter_mut().enumerate() {
                        // The between-ELEMENTS cancellation point: a cold
                        // fan has no cost stats yet, so its chunks are
                        // sized by the cost cap (25 elements) — without
                        // this check an Esc mid-chunk drained up to 25 ×
                        // one element's wall time (stage-6 measurement:
                        // the wall carve's cold Esc was bounded by it).
                        // Unfilled slots stay None → Cancelled above.
                        if self.token.is_cancelled() {
                            break;
                        }
                        let index = start + offset;
                        *slot = Some(self.run_element(
                            decl,
                            inputs,
                            fanned,
                            index,
                            element_cache,
                            executed,
                        ));
                        done += 1;
                    }
                    let nanos = clock.now_nanos().saturating_sub(begin);
                    total_nanos.fetch_add(nanos, Ordering::Relaxed);
                    self.observer.on_event(&Event::ChunkExecuted {
                        node: &decl.name,
                        start,
                        len: done,
                        nanos,
                    });
                });
            }
        });
        let nanos = total_nanos.load(Ordering::Relaxed);
        let computed = executed.load(Ordering::Relaxed);
        (results, nanos, computed)
    }

    /// One element of a fan-out: element key → element memo → run.
    /// Increments `executed` only when the run function actually runs.
    fn run_element(
        &self,
        decl: &NodeDecl,
        inputs: &[Option<Arc<HashedValue>>],
        fanned: &[(usize, &List)],
        index: usize,
        element_cache: bool,
        executed: &AtomicU64,
    ) -> Result<Vec<Arc<HashedValue>>, (usize, String)> {
        // Element inputs: fanned ports take slot `index` (holes were
        // refused above), broadcast ports pass through.
        let mut element_inputs: Vec<Option<Arc<HashedValue>>> = inputs.to_vec();
        for (slot, list) in fanned {
            element_inputs[*slot].clone_from(&list.slots[index]);
        }

        let flat_fan = vec![0_u8; decl.fan.len()];
        let element_key = element_cache.then(|| {
            let hashes: Vec<Option<ValueHash>> = element_inputs
                .iter()
                .map(|value| value.as_ref().map(|value| value.hash()))
                .collect();
            node_key(&KeyInputs {
                op: &decl.op,
                version: decl.version,
                body_hash: decl.body_hash.as_ref(),
                tolerance: decl.tolerance.as_ref(),
                inputs: &hashes,
                fan: &flat_fan,
            })
        });
        if let Some(key) = element_key
            && let Some(entry) = self.scheduler.store.memo(&key)
        {
            // An element entry with the wrong arity, or one whose blobs
            // fail to load, broke its promise: tombstone it and fall
            // through to compute — the cache never owes correctness
            // (docs/12), and recomputing re-stores good bytes.
            if entry.outputs.len() == decl.output_count {
                match self.load_element_hit(decl, key, &entry, index) {
                    Ok(Some(outputs)) => return Ok(outputs),
                    Ok(None) => {} // broken promise, tombstoned — compute
                    Err(failure) => return Err(failure),
                }
            } else if let Err(error) = self.scheduler.store.invalidate_memo(key) {
                self.set_fatal(error);
                return Err((index, "store failure".to_owned()));
            }
        }

        executed.fetch_add(1, Ordering::Relaxed);
        let ctx = NodeCtx { cancel: self.token };
        let run_result = catch_unwind(AssertUnwindSafe(|| (decl.run)(&ctx, &element_inputs)));
        let outputs = match run_result {
            Err(payload) => return Err((index, panic_message(payload.as_ref()))),
            Ok(Err(error)) => return Err((index, error.message)),
            Ok(Ok(outputs)) => outputs,
        };
        if outputs.len() != decl.output_count {
            return Err((
                index,
                format!(
                    "node returned {} outputs; its spec declares {}",
                    outputs.len(),
                    decl.output_count
                ),
            ));
        }
        if let Some(key) = element_key {
            for value in &outputs {
                if let Err(error) = self.scheduler.store.store_value(value) {
                    self.set_fatal(error);
                    return Err((index, "store failure".to_owned()));
                }
            }
            let hashes: Vec<ValueHash> = outputs.iter().map(|value| value.hash()).collect();
            if let Err(error) = self.scheduler.store.record_memo(key, &hashes) {
                self.set_fatal(error);
                return Err((index, "store failure".to_owned()));
            }
        }
        Ok(outputs)
    }

    /// Load an element-memo hit's outputs. `Ok(Some)` = served from cache;
    /// `Ok(None)` = the entry's promise broke (unloadable blob) — it was
    /// tombstoned and the caller recomputes; `Err` = fatal store trouble.
    fn load_element_hit(
        &self,
        decl: &NodeDecl,
        key: NodeKey,
        entry: &MemoEntry,
        index: usize,
    ) -> Result<Option<Vec<Arc<HashedValue>>>, (usize, String)> {
        let mut outputs = Vec::with_capacity(entry.outputs.len());
        for hash in &entry.outputs {
            match self.scheduler.store.load_value(hash) {
                Ok(value) => outputs.push(value),
                Err(
                    StoreError::MissingValue { .. }
                    | StoreError::Decode { .. }
                    | StoreError::CorruptValue { .. }
                    | StoreError::ValueRejected { .. },
                ) => {
                    // Broken promise (blob quarantined or gone): tombstone
                    // and recompute — self-heal within this very solve.
                    if let Err(error) = self.scheduler.store.invalidate_memo(key) {
                        self.set_fatal(error);
                        return Err((index, "store failure".to_owned()));
                    }
                    return Ok(None);
                }
                Err(error) => {
                    // Genuine I/O trouble is fatal, not healable.
                    self.set_fatal(error);
                    return Err((index, "store failure".to_owned()));
                }
            }
        }
        self.observer.on_event(&Event::ElementCacheHit {
            node: &decl.name,
            index,
        });
        Ok(Some(outputs))
    }

    /// Assemble per-port output lists from element outputs and commit.
    #[allow(clippy::too_many_arguments)] // one call site; splitting hurts
    fn finish_fanned(
        &self,
        id: NodeId,
        decl: &NodeDecl,
        axis: Option<&Arc<str>>,
        element_outputs: &[Vec<Arc<HashedValue>>],
        nanos: u64,
        computed: u64,
    ) -> Computed {
        let elements = u64::try_from(element_outputs.len()).unwrap_or(u64::MAX);
        let mut outputs = Vec::with_capacity(decl.output_count);
        for port in 0..decl.output_count {
            let slots = element_outputs
                .iter()
                .map(|element| Some(Arc::clone(&element[port])))
                .collect();
            match HashedValue::new(ValueData::List(List {
                axis: axis.cloned(),
                slots,
            })) {
                Ok(list) => outputs.push(list),
                Err(error) => {
                    return Computed::Failed(NodeFailure {
                        node: decl.name.clone(),
                        message: error.to_string(),
                        element_ids: Vec::new(),
                    });
                }
            }
        }
        match self.commit_outputs(id, outputs) {
            Err(error) => {
                self.set_fatal(error);
                Computed::Cancelled
            }
            Ok(hashes) => Computed::Done {
                outputs: hashes,
                elements,
                computed,
                nanos,
            },
        }
    }
}

/// Validate a fan-out's shape: every fanned input is a list, lengths zip
/// strictly (counts in the error, docs/09), and no absent `Optional` slots
/// (slot-preserving nulls never silently skip — `compact` removes holes).
fn fanned_shape<'v>(
    decl: &NodeDecl,
    inputs: &'v [Option<Arc<HashedValue>>],
) -> Result<FannedInputs<'v>, NodeFailure> {
    let failure = |message: String, element_ids: Vec<usize>| NodeFailure {
        node: decl.name.clone(),
        message,
        element_ids,
    };
    let mut fanned: Vec<(usize, &List)> = Vec::new();
    for (slot, (&depth, input)) in decl.fan.iter().zip(inputs).enumerate() {
        if depth == 0 {
            continue;
        }
        let Some(value) = input else {
            return Err(failure("each() on an absent input".to_owned(), Vec::new()));
        };
        let ValueData::List(list) = value.data() else {
            return Err(failure(
                format!(
                    "each() needs a list; input {slot} is {}",
                    value.data().kind_name()
                ),
                Vec::new(),
            ));
        };
        fanned.push((slot, list));
    }
    let n = fanned.first().map_or(0, |(_, list)| list.slots.len());
    for (slot, list) in &fanned {
        if list.slots.len() != n {
            return Err(failure(
                format!(
                    "zip is strict: {n} vs {} elements (inputs {} and {slot})",
                    list.slots.len(),
                    fanned[0].0,
                ),
                Vec::new(),
            ));
        }
    }
    let mut holes: Vec<usize> = fanned
        .iter()
        .flat_map(|(_, list)| {
            list.slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.is_none())
                .map(|(index, _)| index)
        })
        .collect();
    if !holes.is_empty() {
        holes.sort_unstable();
        holes.dedup();
        return Err(failure(
            "each() input has absent Optional slots — `compact` removes the holes".to_owned(),
            holes,
        ));
    }
    Ok((fanned, n))
}

/// Best-effort text of a caught panic payload. Public since stage 5: the
/// server's generation loop wraps `solve` in the same `catch_unwind`
/// discipline as [`crate::preview::PreviewSession`].
#[must_use]
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "node panicked".to_owned()
    }
}
