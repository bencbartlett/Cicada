//! The session's generation loop (docs/12 §Solve generations, docs/13
//! §Slider drags): ONE worker running latest-wins generations over the
//! shared scheduler. A submission replaces whatever is pending; each
//! completed generation immediately starts the next with the newest job.
//! Two policies, per docs/12: **structural** submissions (edits, reloads)
//! cancel and supersede the in-flight generation; **preview** submissions
//! (slider streams) let the in-flight generation COMPLETE — its work lands
//! in the store, and killing Python per tick would mean a cone through a
//! script node never produces a preview at all. Esc (`cancel`) cancels
//! whatever is running.
//!
//! Every generation owns its `CancelToken` (v0.1 item 3b): cancelling it is
//! the whole of cancelling the generation — the executor checks it between
//! nodes and chunks, and the script bridge's per-call kill switches are
//! hooked to it (`CancelToken::on_cancel`), so there is no separate "kill
//! the scripts" step for any cancel site to forget. Explicit effectful runs
//! (`POST /api/run/{node}`) do NOT go through this loop: they solve on the
//! same scheduler with their own token, so a slider drag never cancels an
//! export half-written (see [`crate::session`]) — by construction.
//!
//! **Idle class** ([`SolveLoop::run_idle`]): a hypothetical solve — the
//! substrate of scrub caching and `cycle` warming (docs/12 §Speculative
//! warming) — waits until the loop is idle, solves on the caller's thread
//! with its own token, and is pre-empted (cancelled) by ANY real submission
//! or Esc. It is never "in flight" as far as `wait_idle`/`is_busy` are
//! concerned, and it reports through the caller's observer only — the
//! session paints nothing for it. Its completed work lands in the ordinary
//! memo, which is the point.
//!
//! Wall-clock policy (the ~30 ms structural debounce) is the session's, not
//! this loop's — `param_preview` streams submit immediately, structural
//! edits arrive here after the session's timer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cicada_sched::{CancelToken, Event, NodeId, Observer, Scheduler, SolveError, SolveReport};

use crate::lower::Lowered;

/// Why a generation ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// A structural edit / load / reload.
    Structural,
    /// A `param_preview` stream value (ephemeral text state).
    Preview,
    /// A transport frame (docs/13 §Animation transport): the playhead's
    /// values injected into the time params at lowering. A preview for
    /// this loop's purposes — the in-flight generation completes, the
    /// newest frame replaces a queued one — with its own timing kind.
    Transport,
}

/// One generation's work.
pub struct Job {
    /// The lowered graph (owned by the job so a superseded job drops it).
    pub lowered: Arc<Lowered>,
    /// Pull targets.
    pub targets: Vec<NodeId>,
    /// Why.
    pub kind: JobKind,
    /// When the session accepted the work this job carries — a preview:
    /// the `param_preview` intent's arrival (BEFORE its lowering, so the
    /// queue time covers everything between the message and the solve
    /// starting); structural: the debounce firing / the load. The
    /// generation's `queued_ms` (docs/15 preview-latency currency) is
    /// start − this.
    pub submitted: Instant,
}

/// The session's hooks — called on the loop thread (events also arrive on
/// rayon worker threads via the observer). Implementations must be cheap
/// and must never call back into [`SolveLoop::submit`] while holding locks
/// the loop needs (they need none: the loop holds no lock while running a
/// generation or calling these).
pub trait SolveSink: Send + Sync {
    /// A generation is starting.
    fn on_start(&self, generation: u64, job: &Job);
    /// An execution event (worker threads).
    fn on_event(&self, generation: u64, event: &Event<'_>);
    /// A generation finished (cancelled or not).
    fn on_complete(&self, generation: u64, job: &Job, report: Arc<SolveReport>);
    /// A generation failed at the engine level.
    fn on_error(&self, generation: u64, job: &Job, error: &SolveError);
    /// An explicit [`SolveLoop::cancel`] (Esc) ended this generation and
    /// the loop is idle again: `cancel_to_idle` is the server-side wall
    /// time from the FIRST `cancel()` call during the generation to the
    /// loop flipping idle (after `on_complete`/`on_error` returned — frame
    /// emission included). Structural supersession is not a cancel in this
    /// sense and never reports here. Called after `on_complete`.
    fn on_cancel_settled(&self, generation: u64, cancel_to_idle: Duration);
}

struct State {
    pending: Option<Job>,
    current: Option<CancelToken>,
    in_flight: bool,
    /// The first explicit `cancel()` during the in-flight generation.
    cancel_at: Option<Instant>,
    shutdown: bool,
    /// Tokens of idle-class solves running right now ([`SolveLoop::
    /// run_idle`]); every real submission and every Esc cancels them all.
    idle: Vec<CancelToken>,
    /// Idle-class solves waiting for the loop to go idle (registered
    /// under the lock before they wait, deregistered before they run or
    /// return): the state tests assert "it has not started" by, instead
    /// of by elapsed time.
    idle_waiting: usize,
}

struct Shared {
    scheduler: Arc<Scheduler>,
    sink: Arc<dyn SolveSink>,
    state: Mutex<State>,
    wake: Condvar,
    generation: AtomicU64,
}

/// Why an idle-class solve returned no report.
#[derive(Debug, thiserror::Error)]
pub enum IdleError {
    /// The loop shut down while the idle solve waited for its turn.
    #[error("the solve loop shut down before the idle solve could start")]
    Shutdown,
    /// The scheduler refused (store I/O, engine panic) — same as for any
    /// generation.
    #[error(transparent)]
    Solve(#[from] SolveError),
}

/// An idle-class solve's result: its generation number (unique per
/// session, from the shared counter) and the report — `cancelled` when a
/// real generation pre-empted it.
#[derive(Debug)]
pub struct IdleRun {
    /// The generation number the solve ran under.
    pub generation: u64,
    /// The scheduler's report.
    pub report: SolveReport,
}

/// The latest-wins generation loop.
pub struct SolveLoop {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

struct GenerationObserver {
    generation: u64,
    sink: Arc<dyn SolveSink>,
}

impl Observer for GenerationObserver {
    fn on_event(&self, event: &Event<'_>) {
        self.sink.on_event(self.generation, event);
    }
}

impl SolveLoop {
    /// Start the loop.
    ///
    /// # Panics
    ///
    /// When the worker thread cannot spawn — a loop without its worker
    /// would swallow every submit silently.
    #[must_use]
    pub fn new(scheduler: Arc<Scheduler>, sink: Arc<dyn SolveSink>) -> Self {
        let shared = Arc::new(Shared {
            scheduler,
            sink,
            state: Mutex::new(State {
                pending: None,
                current: None,
                in_flight: false,
                cancel_at: None,
                shutdown: false,
                idle: Vec::new(),
                idle_waiting: 0,
            }),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = match std::thread::Builder::new()
            .name("cicada-solve".to_owned())
            .spawn(move || worker_loop(&worker_shared))
        {
            Ok(handle) => Some(handle),
            Err(error) => panic!("solve worker thread could not spawn: {error}"),
        };
        Self { shared, worker }
    }

    /// Submit the newest job: replaces any pending job. A structural job
    /// also cancels the in-flight generation; a preview job lets it finish
    /// (latest-wins over COMPLETED generations, docs/12). Any real job
    /// pre-empts every idle-class solve.
    pub fn submit(&self, job: Job) {
        let mut state = self.lock();
        let structural = job.kind == JobKind::Structural;
        state.pending = Some(job);
        if structural && let Some(token) = &state.current {
            token.cancel();
        }
        preempt_idle(&mut state);
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Cancel (Esc): the in-flight generation is cancelled AND a pending
    /// job is dropped — from the user's seat "Esc" means "stop solving",
    /// not "stop this one and start the next"; the next edit (or a
    /// preview tick) resubmits. Idle-class solves stop too. Returns true
    /// when a pending job was dropped.
    #[must_use]
    pub fn cancel(&self) -> bool {
        let mut state = self.lock();
        if let Some(token) = &state.current {
            token.cancel();
            // The first Esc during this generation starts the
            // cancel-to-idle clock (a second Esc is the user hammering the
            // key — the honest latency is from the first).
            state.cancel_at.get_or_insert_with(Instant::now);
        }
        preempt_idle(&mut state);
        state.pending.take().is_some()
    }

    /// Run an idle-class solve on the CALLING thread (docs/12 §Speculative
    /// warming — "always at the lowest priority, preempted by any real
    /// work"): blocks until the loop has nothing pending or in flight,
    /// then solves `targets` of `lowered` under a fresh token that every
    /// later [`Self::submit`] / [`Self::cancel`] cancels. Events go to
    /// `observer` and nowhere else — the session's sink never hears of it,
    /// `wait_idle` and `is_busy` ignore it, and its generation number comes
    /// from the shared counter so timings stay unique. Completed work is in
    /// the memo when this returns, pre-empted or not.
    ///
    /// # Errors
    ///
    /// [`IdleError::Shutdown`] when the loop shuts down while the solve
    /// waits; [`IdleError::Solve`] on an engine-level failure.
    pub fn run_idle(
        &self,
        lowered: &Lowered,
        targets: &[NodeId],
        observer: &dyn Observer,
    ) -> Result<IdleRun, IdleError> {
        let (token, generation) = {
            let mut state = self.lock();
            state.idle_waiting += 1;
            let mut state = self
                .shared
                .wake
                .wait_while(state, |state| {
                    (state.in_flight || state.pending.is_some()) && !state.shutdown
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.idle_waiting -= 1;
            if state.shutdown {
                return Err(IdleError::Shutdown);
            }
            // Registered under the same lock hold that observed "idle": a
            // submit cannot slip between the check and the registration.
            let token = CancelToken::new();
            state.idle.push(token.clone());
            let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            (token, generation)
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.shared
                .scheduler
                .solve(&lowered.graph, targets, generation, &token, observer)
        }))
        .unwrap_or_else(|payload| {
            Err(SolveError::EnginePanic {
                message: cicada_sched::exec::panic_message(payload.as_ref()),
            })
        });
        {
            let mut state = self.lock();
            state.idle.retain(|idle| !idle.same(&token));
        }
        let report = result?;
        Ok(IdleRun { generation, report })
    }

    /// Idle-class solves running right now (tests; diagnostics).
    #[must_use]
    pub fn idle_in_flight(&self) -> usize {
        self.lock().idle.len()
    }

    /// Idle-class solves waiting for their turn (tests; diagnostics).
    #[must_use]
    pub fn idle_waiting(&self) -> usize {
        self.lock().idle_waiting
    }

    /// Is a generation in flight (or queued)?
    #[must_use]
    pub fn is_busy(&self) -> bool {
        let state = self.lock();
        state.in_flight || state.pending.is_some()
    }

    /// Block until idle (tests and shutdown).
    pub fn wait_idle(&self) {
        let state = self.lock();
        let _idle = self
            .shared
            .wake
            .wait_while(state, |state| state.in_flight || state.pending.is_some())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    /// The next generation number (shared with explicit runs so every
    /// generation number is unique per session).
    #[must_use]
    pub fn next_generation(&self) -> u64 {
        self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The scheduler.
    #[must_use]
    pub fn scheduler(&self) -> &Arc<Scheduler> {
        &self.shared.scheduler
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SolveLoop {
    /// The shutdown half that needs no ownership: flag it, cancel whatever
    /// runs (the in-flight generation and every idle-class solve), wake
    /// every waiter — idle solves waiting for their turn return
    /// [`IdleError::Shutdown`]. `Drop` calls this and then joins the worker.
    fn begin_shutdown(&self) {
        {
            let mut state = self.lock();
            state.shutdown = true;
            if let Some(token) = &state.current {
                token.cancel();
            }
            preempt_idle(&mut state);
        }
        self.shared.wake.notify_all();
    }
}

impl Drop for SolveLoop {
    fn drop(&mut self) {
        self.begin_shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Cancel every idle-class solve in flight: real work arrived (or Esc).
/// The tokens stay registered until their solves return and deregister
/// themselves — cancelling twice is harmless.
fn preempt_idle(state: &mut State) {
    for token in &state.idle {
        token.cancel();
    }
}

fn worker_loop(shared: &Shared) {
    loop {
        let (job, token, generation) = {
            let state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut state = shared
                .wake
                .wait_while(state, |state| state.pending.is_none() && !state.shutdown)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.shutdown {
                return;
            }
            let Some(job) = state.pending.take() else {
                continue;
            };
            let token = CancelToken::new();
            state.current = Some(token.clone());
            state.in_flight = true;
            let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
            (job, token, generation)
        };
        shared.sink.on_start(generation, &job);
        let observer = GenerationObserver {
            generation,
            sink: Arc::clone(&shared.sink),
        };
        // catch_unwind: a panic inside solve (an observer, an executor
        // invariant) must not kill the loop thread — that would wedge the
        // session silently, the opposite of fail-loudly. It becomes an
        // engine error the session reports.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.scheduler.solve(
                &job.lowered.graph,
                &job.targets,
                generation,
                &token,
                &observer,
            )
        }))
        .unwrap_or_else(|payload| {
            Err(SolveError::EnginePanic {
                message: cicada_sched::exec::panic_message(payload.as_ref()),
            })
        });
        let solve_returned = Instant::now();
        match result {
            Ok(report) => shared.sink.on_complete(generation, &job, Arc::new(report)),
            Err(error) => shared.sink.on_error(generation, &job, &error),
        }
        if std::env::var_os("CICADA_TRACE").is_some_and(|v| !v.is_empty()) {
            eprintln!(
                "trace: generation {generation} completion hooks {:.3} ms",
                solve_returned.elapsed().as_secs_f64() * 1000.0
            );
        }
        let cancel_to_idle = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Measured at the idle flip itself; `is_busy` pollers see idle
            // the moment this lock drops.
            let cancel_to_idle = state.cancel_at.take().map(|at| at.elapsed());
            state.in_flight = false;
            state.current = None;
            cancel_to_idle
        };
        if let Some(cancel_to_idle) = cancel_to_idle {
            shared.sink.on_cancel_settled(generation, cancel_to_idle);
        }
        shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_sched::{
        DiskStore, Input, MonotonicClock, NodeDecl, NodeOutcome, SchedulerConfig, SolveGraph,
    };
    use std::collections::{BTreeMap, HashMap};
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct Recorder {
        starts: AtomicUsize,
        completes: Mutex<Vec<(u64, bool)>>,
        events: AtomicUsize,
        settled: AtomicUsize,
    }

    impl SolveSink for Recorder {
        fn on_start(&self, _generation: u64, _job: &Job) {
            self.starts.fetch_add(1, Ordering::SeqCst);
        }
        fn on_event(&self, _generation: u64, _event: &Event<'_>) {
            self.events.fetch_add(1, Ordering::SeqCst);
        }
        fn on_complete(&self, generation: u64, _job: &Job, report: Arc<SolveReport>) {
            self.completes
                .lock()
                .unwrap()
                .push((generation, report.cancelled));
        }
        fn on_error(&self, _generation: u64, _job: &Job, error: &SolveError) {
            panic!("engine error: {error}");
        }
        fn on_cancel_settled(&self, _generation: u64, _cancel_to_idle: Duration) {
            self.settled.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn graph(x: f64) -> Arc<Lowered> {
        let run: cicada_sched::NodeFn = Arc::new(|_ctx, inputs: &[Option<Arc<HashedValue>>]| {
            let a = inputs[0].as_ref().unwrap();
            let ValueData::Number(a) = a.data() else {
                panic!()
            };
            Ok(vec![HashedValue::new(ValueData::Number(a * 2.0)).unwrap()])
        });
        let decl = NodeDecl {
            name: "twice".to_owned(),
            op: "twice".to_owned(),
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: vec![Input::Value(
                HashedValue::new(ValueData::Number(x)).unwrap(),
            )],
            fan: vec![0],
            output_count: 1,
            effectful: false,
            volatile: false,
            run,
        };
        Arc::new(Lowered {
            graph: SolveGraph::new(vec![decl]).unwrap(),
            bindings: HashMap::new(),
            output_names: vec![vec!["out".to_owned()]],
            excluded: BTreeMap::new(),
            driven: Vec::new(),
        })
    }

    #[test]
    fn generations_run_latest_wins_and_report() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(store),
                Arc::new(MonotonicClock::new()),
                SchedulerConfig {
                    threads: 1,
                    ..SchedulerConfig::default()
                },
            )
            .unwrap(),
        );
        let recorder = Arc::new(Recorder {
            starts: AtomicUsize::new(0),
            completes: Mutex::new(Vec::new()),
            events: AtomicUsize::new(0),
            settled: AtomicUsize::new(0),
        });
        let solve = SolveLoop::new(scheduler, recorder.clone());
        for x in 0..20 {
            solve.submit(Job {
                lowered: graph(f64::from(x)),
                targets: vec![NodeId(0)],
                kind: JobKind::Preview,
                submitted: Instant::now(),
            });
        }
        solve.wait_idle();
        let completes = recorder.completes.lock().unwrap().clone();
        assert!(!completes.is_empty());
        assert!(completes.len() <= 20, "superseded jobs never run");
        // The LAST completed generation is the newest value and complete.
        let (last_generation, cancelled) = *completes.last().unwrap();
        assert!(!cancelled, "the final generation runs to completion");
        assert_eq!(recorder.starts.load(Ordering::SeqCst), completes.len());
        assert!(recorder.events.load(Ordering::SeqCst) > 0);
        assert!(solve.next_generation() > last_generation);
        assert!(!solve.is_busy());
        // A one-off submit reports Computed/CacheHit for the node.
        solve.submit(Job {
            lowered: graph(19.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        solve.wait_idle();
        let completes = recorder.completes.lock().unwrap();
        assert!(completes.len() >= 2);
        drop(completes);
        assert_eq!(
            recorder.settled.load(Ordering::SeqCst),
            0,
            "supersession is not an Esc: no cancel-to-idle record without cancel()"
        );
        drop(solve);
    }

    struct Last {
        report: Mutex<Option<Arc<SolveReport>>>,
        /// `(generation, cancel → idle)` per Esc-ended generation.
        settled: Mutex<Vec<(u64, Duration)>>,
    }
    impl Last {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                report: Mutex::new(None),
                settled: Mutex::new(Vec::new()),
            })
        }
    }
    impl SolveSink for Last {
        fn on_start(&self, _: u64, _: &Job) {}
        fn on_event(&self, _: u64, _: &Event<'_>) {}
        fn on_complete(&self, _: u64, _: &Job, report: Arc<SolveReport>) {
            *self.report.lock().unwrap() = Some(report);
        }
        fn on_error(&self, _: u64, _: &Job, error: &SolveError) {
            panic!("{error}");
        }
        fn on_cancel_settled(&self, generation: u64, cancel_to_idle: Duration) {
            self.settled
                .lock()
                .unwrap()
                .push((generation, cancel_to_idle));
        }
    }

    #[test]
    fn preview_submits_let_the_in_flight_generation_complete() {
        // docs/12: continuous param streams run latest-wins over COMPLETED
        // generations — the in-flight one is never cancelled (killing a
        // Python node per tick would mean no preview ever lands).
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(store),
                Arc::new(MonotonicClock::new()),
                SchedulerConfig {
                    threads: 1,
                    ..SchedulerConfig::default()
                },
            )
            .unwrap(),
        );
        let last = Last::new();
        let runs = Arc::new(AtomicUsize::new(0));
        // The FIRST run blocks on this gate until the test has streamed the
        // newer previews — no sleeps, no wall clock: the outcome is the
        // same on a loaded CI runner as on the dev machine.
        let gate = Arc::new(AtomicBool::new(false));
        let solve = SolveLoop::new(scheduler, last.clone());
        let slow = |x: f64, runs: Arc<AtomicUsize>, gate: Arc<AtomicBool>| -> Arc<Lowered> {
            let run: cicada_sched::NodeFn =
                Arc::new(move |_ctx, inputs: &[Option<Arc<HashedValue>>]| {
                    if runs.fetch_add(1, Ordering::SeqCst) == 0 {
                        while !gate.load(Ordering::SeqCst) {
                            std::thread::yield_now();
                        }
                    }
                    let a = inputs[0].as_ref().unwrap();
                    let ValueData::Number(a) = a.data() else {
                        panic!()
                    };
                    Ok(vec![HashedValue::new(ValueData::Number(a + 1.0)).unwrap()])
                });
            let decl = NodeDecl {
                name: "slow".to_owned(),
                op: "slow".to_owned(),
                version: 1,
                body_hash: None,
                tolerance: None,
                inputs: vec![Input::Value(
                    HashedValue::new(ValueData::Number(x)).unwrap(),
                )],
                fan: vec![0],
                output_count: 1,
                effectful: false,
                volatile: false,
                run,
            };
            Arc::new(Lowered {
                graph: SolveGraph::new(vec![decl]).unwrap(),
                bindings: HashMap::new(),
                output_names: vec![vec!["out".to_owned()]],
                excluded: BTreeMap::new(),
                driven: Vec::new(),
            })
        };
        solve.submit(Job {
            lowered: slow(1.0, runs.clone(), gate.clone()),
            targets: vec![NodeId(0)],
            kind: JobKind::Preview,
            submitted: Instant::now(),
        });
        // Wait until the first generation is RUNNING (its node holds the
        // gate), then stream newer previews: none may cancel it.
        while runs.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        for x in 2..6 {
            solve.submit(Job {
                lowered: slow(f64::from(x), runs.clone(), gate.clone()),
                targets: vec![NodeId(0)],
                kind: JobKind::Preview,
                submitted: Instant::now(),
            });
        }
        gate.store(true, Ordering::SeqCst);
        solve.wait_idle();
        let report = last.report.lock().unwrap().clone().unwrap();
        assert!(!report.cancelled, "the newest preview ran to completion");
        // The first generation completed (1 run) and the newest ran (1 run);
        // intermediates were superseded before starting: exactly 2 runs.
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cancel_marks_the_generation_cancelled_or_finishes_fast_work() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(store),
                Arc::new(MonotonicClock::new()),
                SchedulerConfig {
                    threads: 1,
                    ..SchedulerConfig::default()
                },
            )
            .unwrap(),
        );
        let last = Last::new();
        let solve = SolveLoop::new(scheduler, last.clone());
        solve.submit(Job {
            lowered: graph(1.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        let dropped = solve.cancel();
        solve.wait_idle();
        let report = last.report.lock().unwrap().clone();
        // Three honest outcomes, none wedged: the cancel dropped the job
        // before it started (no report), or it landed while the node ran
        // (Cancelled), or the node was too fast (Computed).
        match report {
            None => assert!(dropped, "no report means the pending job was dropped"),
            Some(report) => assert!(matches!(
                report.outcome(NodeId(0)),
                NodeOutcome::Cancelled
                    | NodeOutcome::Computed { .. }
                    | NodeOutcome::CacheHit { .. }
            )),
        }
    }

    #[test]
    fn esc_mid_generation_reports_cancel_to_idle_once_for_that_generation() {
        // Deterministic: the node blocks on a channel until the test has
        // called cancel() (no sleeps, no wall-clock races) — so the Esc
        // provably lands mid-generation, and the loop reports exactly one
        // cancel → idle duration for exactly that generation, after the
        // completion hook.
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(store),
                Arc::new(MonotonicClock::new()),
                SchedulerConfig {
                    threads: 1,
                    ..SchedulerConfig::default()
                },
            )
            .unwrap(),
        );
        let last = Last::new();
        let solve = SolveLoop::new(scheduler, last.clone());
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let run: cicada_sched::NodeFn =
            Arc::new(move |_ctx, _inputs: &[Option<Arc<HashedValue>>]| {
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                Ok(vec![HashedValue::new(ValueData::Number(1.0)).unwrap()])
            });
        let decl = NodeDecl {
            name: "gate".to_owned(),
            op: "gate".to_owned(),
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: vec![],
            fan: vec![],
            output_count: 1,
            effectful: false,
            volatile: false,
            run,
        };
        let lowered = Arc::new(Lowered {
            graph: SolveGraph::new(vec![decl]).unwrap(),
            bindings: HashMap::new(),
            output_names: vec![vec!["out".to_owned()]],
            excluded: BTreeMap::new(),
            driven: Vec::new(),
        });
        solve.submit(Job {
            lowered,
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        started_rx.recv().unwrap();
        assert!(solve.is_busy());
        let dropped = solve.cancel();
        assert!(
            !dropped,
            "nothing was pending — the Esc hit the in-flight generation"
        );
        let dropped_again = solve.cancel();
        assert!(!dropped_again);
        release_tx.send(()).unwrap();
        solve.wait_idle();
        let report = last.report.lock().unwrap().clone().unwrap();
        assert!(report.cancelled, "the generation ended cancelled");
        let settled = last.settled.lock().unwrap().clone();
        assert_eq!(settled.len(), 1, "two Esc presses, one record: {settled:?}");
        assert_eq!(
            settled[0].0, 1,
            "the first generation is the one the Esc ended"
        );
        assert!(settled[0].1 > Duration::ZERO);
        // The next generation runs clean: no stale cancel clock leaks.
        solve.submit(Job {
            lowered: graph(2.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        solve.wait_idle();
        assert_eq!(last.settled.lock().unwrap().len(), 1);
        assert!(!last.report.lock().unwrap().clone().unwrap().cancelled);
    }
    // ------------------------------------------------------- idle class --

    /// A gated one-node graph: the node signals `started` and holds until
    /// `release` fires OR its generation's token is cancelled (polled, the
    /// way a host bridge would) — the deterministic way to hold a
    /// generation open without ever wedging a shutdown.
    fn gated(
        name: &str,
        x: f64,
    ) -> (
        Arc<Lowered>,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let run: cicada_sched::NodeFn =
            Arc::new(move |ctx, _inputs: &[Option<Arc<HashedValue>>]| {
                started_tx.send(()).unwrap();
                let release = release_rx.lock().unwrap();
                while !ctx.cancel.is_cancelled() {
                    if release.recv_timeout(Duration::from_millis(1)).is_ok() {
                        break;
                    }
                }
                Ok(vec![HashedValue::new(ValueData::Number(x)).unwrap()])
            });
        let decl = NodeDecl {
            name: name.to_owned(),
            op: name.to_owned(),
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: vec![Input::Value(
                HashedValue::new(ValueData::Number(x)).unwrap(),
            )],
            fan: vec![0],
            output_count: 1,
            effectful: false,
            volatile: false,
            run,
        };
        let lowered = Arc::new(Lowered {
            graph: SolveGraph::new(vec![decl]).unwrap(),
            bindings: HashMap::new(),
            output_names: vec![vec!["out".to_owned()]],
            excluded: BTreeMap::new(),
            driven: Vec::new(),
        });
        (lowered, started_rx, release_tx)
    }

    fn fresh_loop(sink: Arc<dyn SolveSink>) -> SolveLoop {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        // The store outlives the loop through the scheduler's Arc; the
        // directory is leaked on purpose for the test's lifetime.
        std::mem::forget(dir);
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(store),
                Arc::new(MonotonicClock::new()),
                SchedulerConfig {
                    threads: 2,
                    ..SchedulerConfig::default()
                },
            )
            .unwrap(),
        );
        SolveLoop::new(scheduler, sink)
    }

    #[test]
    fn idle_solve_waits_for_the_loop_then_is_preempted_by_real_work() {
        // Deterministic (channels, no sleeps): a structural generation is
        // held open; an idle solve submitted meanwhile must not start until
        // it completes; once running, it is invisible to is_busy/wait_idle;
        // a preview submission pre-empts it (its token is cancelled), and
        // the loop's own generation still runs clean.
        let last = Last::new();
        let solve = Arc::new(fresh_loop(last.clone()));
        let (held, held_started, held_release) = gated("held", 1.0);
        solve.submit(Job {
            lowered: held,
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        held_started.recv().unwrap();
        assert!(solve.is_busy());

        let (idle_graph, idle_started, idle_release) = gated("idle", 2.0);
        let idle = {
            let solve = Arc::clone(&solve);
            std::thread::spawn(move || {
                solve
                    .run_idle(&idle_graph, &[NodeId(0)], &cicada_sched::NoopObserver)
                    .unwrap()
            })
        };
        // While the real generation is held, the idle solve has not begun.
        // Asserted by STATE, not by elapsed time: the idle solve registers
        // as waiting under the loop's lock and only then waits (releasing
        // the lock) — so observing `idle_waiting() == 1` means it is parked
        // on the condvar, with nothing idle registered and its node silent.
        // A loop that did not wait (or waited on the wrong predicate)
        // would never show 1 from outside (increment and decrement under
        // one lock hold) and would signal `started` instead — the loop
        // below fails on that signal rather than spinning.
        loop {
            if solve.idle_waiting() == 1 {
                break;
            }
            assert!(
                idle_started.try_recv().is_err(),
                "the idle solve must not start while a generation is in flight"
            );
            std::thread::yield_now();
        }
        assert!(idle_started.try_recv().is_err());
        assert_eq!(solve.idle_in_flight(), 0);

        // Release the real generation: the loop goes idle, the idle solve
        // starts — and is NOT "busy".
        held_release.send(()).unwrap();
        idle_started.recv().unwrap();
        assert_eq!(solve.idle_in_flight(), 1);
        assert_eq!(solve.idle_waiting(), 0, "it left the waiting room");
        solve.wait_idle();
        assert!(
            !solve.is_busy(),
            "an idle-class solve is invisible to is_busy/wait_idle"
        );

        // Real work arrives: the idle solve is pre-empted.
        solve.submit(Job {
            lowered: graph(3.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Preview,
            submitted: Instant::now(),
        });
        idle_release.send(()).unwrap();
        let run = idle.join().unwrap();
        assert!(run.report.cancelled, "pre-empted: its token was cancelled");
        assert_eq!(solve.idle_in_flight(), 0, "it deregistered itself");
        solve.wait_idle();
        let report = last.report.lock().unwrap().clone().unwrap();
        assert!(!report.cancelled, "the real generation ran clean");
        assert!(run.generation > 0);
        assert_ne!(
            run.generation, report.generation,
            "generation numbers stay unique"
        );
    }

    #[test]
    fn idle_solve_results_are_cache_hits_for_the_next_real_generation() {
        let last = Last::new();
        let solve = fresh_loop(last.clone());
        let warm = graph(5.0);
        let run = solve
            .run_idle(&warm, &[NodeId(0)], &cicada_sched::NoopObserver)
            .unwrap();
        assert!(!run.report.cancelled);
        assert!(matches!(
            run.report.outcome(NodeId(0)),
            NodeOutcome::Computed { .. }
        ));
        assert!(
            last.report.lock().unwrap().is_none(),
            "the sink never hears of an idle solve"
        );
        solve.submit(Job {
            lowered: graph(5.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        solve.wait_idle();
        let report = last.report.lock().unwrap().clone().unwrap();
        assert!(
            matches!(report.outcome(NodeId(0)), NodeOutcome::CacheHit { .. }),
            "the idle solve's work is in the ordinary memo: {:?}",
            report.outcome(NodeId(0))
        );
    }

    #[test]
    fn idle_solve_refuses_loudly_when_the_loop_shuts_down_first() {
        let last = Last::new();
        let solve = Arc::new(fresh_loop(last.clone()));
        let (held, held_started, held_release) = gated("held", 1.0);
        solve.submit(Job {
            lowered: held,
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
            submitted: Instant::now(),
        });
        held_started.recv().unwrap();
        let waiting = {
            let solve = Arc::clone(&solve);
            std::thread::spawn(move || {
                solve.run_idle(&graph(9.0), &[NodeId(0)], &cicada_sched::NoopObserver)
            })
        };
        // Shut the loop down while the generation is still held and the
        // idle solve waits for its turn: the waiter wakes with Shutdown
        // instead of hanging forever (the held node sees its token
        // cancelled by the shutdown and returns, so the worker can join).
        // `begin_shutdown` is what Drop runs first; calling it directly
        // lets the waiter keep its Arc without keeping the loop alive.
        solve.begin_shutdown();
        let result = waiting.join().unwrap();
        drop(held_release);
        drop(solve);
        assert!(matches!(result, Err(IdleError::Shutdown)), "{result:?}");
    }
}
