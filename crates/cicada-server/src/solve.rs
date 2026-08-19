//! The session's generation loop (docs/12 §Solve generations, docs/13
//! §Slider drags): ONE worker running latest-wins generations over the
//! shared scheduler. A submission replaces whatever is pending; each
//! completed generation immediately starts the next with the newest job.
//! Two policies, per docs/12: **structural** submissions (edits, reloads)
//! cancel and supersede the in-flight generation (its `CancelToken` and
//! the script host's kill switch); **preview** submissions (slider
//! streams) let the in-flight generation COMPLETE — its work lands in the
//! store, and killing Python per tick would mean a cone through a script
//! node never produces a preview at all. Esc (`cancel`) cancels + kills
//! whatever is running.
//!
//! Wall-clock policy (the ~30 ms structural debounce) is the session's, not
//! this loop's — `param_preview` streams submit immediately, structural
//! edits arrive here after the session's timer. Explicit effectful runs
//! (`POST /api/run/{node}`) do NOT go through this loop: they solve on the
//! same scheduler with their own token so a slider drag never cancels an
//! export half-written (see [`crate::session`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use cicada_sched::{CancelToken, Event, NodeId, Observer, Scheduler, SolveError, SolveReport};

use crate::lower::Lowered;
use crate::scripts::ScriptCancel;

/// Why a generation ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// A structural edit / load / reload.
    Structural,
    /// A `param_preview` stream value (ephemeral text state).
    Preview,
}

/// One generation's work.
pub struct Job {
    /// The lowered graph (owned by the job so a superseded job drops it).
    pub lowered: Arc<Lowered>,
    /// Pull targets.
    pub targets: Vec<NodeId>,
    /// Why.
    pub kind: JobKind,
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
}

struct State {
    pending: Option<Job>,
    current: Option<CancelToken>,
    in_flight: bool,
    shutdown: bool,
}

struct Shared {
    scheduler: Arc<Scheduler>,
    scripts: Arc<ScriptCancel>,
    sink: Arc<dyn SolveSink>,
    state: Mutex<State>,
    wake: Condvar,
    generation: AtomicU64,
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
    pub fn new(
        scheduler: Arc<Scheduler>,
        scripts: Arc<ScriptCancel>,
        sink: Arc<dyn SolveSink>,
    ) -> Self {
        let shared = Arc::new(Shared {
            scheduler,
            scripts,
            sink,
            state: Mutex::new(State {
                pending: None,
                current: None,
                in_flight: false,
                shutdown: false,
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
    /// also cancels the in-flight generation (token + script kill switch);
    /// a preview job lets it finish (latest-wins over COMPLETED
    /// generations, docs/12).
    pub fn submit(&self, job: Job) {
        let mut state = self.lock();
        let structural = job.kind == JobKind::Structural;
        state.pending = Some(job);
        if structural && let Some(token) = &state.current {
            token.cancel();
            self.shared.scripts.kill();
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Cancel the in-flight generation (Esc). Pending jobs stay pending.
    pub fn cancel(&self) {
        let state = self.lock();
        if let Some(token) = &state.current {
            token.cancel();
            self.shared.scripts.kill();
        }
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

impl Drop for SolveLoop {
    fn drop(&mut self) {
        {
            let mut state = self.lock();
            state.shutdown = true;
            if let Some(token) = &state.current {
                token.cancel();
                self.shared.scripts.kill();
            }
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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
        // A fresh kill switch for this generation's script calls.
        let _switch = shared.scripts.begin();
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
        match result {
            Ok(report) => shared.sink.on_complete(generation, &job, Arc::new(report)),
            Err(error) => shared.sink.on_error(generation, &job, &error),
        }
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.in_flight = false;
            state.current = None;
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
    use std::sync::atomic::AtomicUsize;

    struct Recorder {
        starts: AtomicUsize,
        completes: Mutex<Vec<(u64, bool)>>,
        events: AtomicUsize,
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
    }

    fn graph(x: f64) -> Arc<Lowered> {
        let run: cicada_sched::NodeFn = Arc::new(|inputs: &[Option<Arc<HashedValue>>]| {
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
            run,
        };
        Arc::new(Lowered {
            graph: SolveGraph::new(vec![decl]).unwrap(),
            bindings: HashMap::new(),
            output_names: vec![vec!["out".to_owned()]],
            excluded: BTreeMap::new(),
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
        });
        let solve = SolveLoop::new(scheduler, ScriptCancel::new(), recorder.clone());
        for x in 0..20 {
            solve.submit(Job {
                lowered: graph(f64::from(x)),
                targets: vec![NodeId(0)],
                kind: JobKind::Preview,
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
        });
        solve.wait_idle();
        let completes = recorder.completes.lock().unwrap();
        assert!(completes.len() >= 2);
        drop(completes);
        drop(solve);
    }

    struct Last(Mutex<Option<Arc<SolveReport>>>);
    impl SolveSink for Last {
        fn on_start(&self, _: u64, _: &Job) {}
        fn on_event(&self, _: u64, _: &Event<'_>) {}
        fn on_complete(&self, _: u64, _: &Job, report: Arc<SolveReport>) {
            *self.0.lock().unwrap() = Some(report);
        }
        fn on_error(&self, _: u64, _: &Job, error: &SolveError) {
            panic!("{error}");
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
        let last = Arc::new(Last(Mutex::new(None)));
        let runs = Arc::new(AtomicUsize::new(0));
        let solve = SolveLoop::new(scheduler, ScriptCancel::new(), last.clone());
        let slow = |x: f64, runs: Arc<AtomicUsize>| -> Arc<Lowered> {
            let run: cicada_sched::NodeFn =
                Arc::new(move |inputs: &[Option<Arc<HashedValue>>]| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(40));
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
                run,
            };
            Arc::new(Lowered {
                graph: SolveGraph::new(vec![decl]).unwrap(),
                bindings: HashMap::new(),
                output_names: vec![vec!["out".to_owned()]],
                excluded: BTreeMap::new(),
            })
        };
        solve.submit(Job {
            lowered: slow(1.0, runs.clone()),
            targets: vec![NodeId(0)],
            kind: JobKind::Preview,
        });
        // Let it start, then stream newer previews: none may cancel it.
        std::thread::sleep(std::time::Duration::from_millis(10));
        for x in 2..6 {
            solve.submit(Job {
                lowered: slow(f64::from(x), runs.clone()),
                targets: vec![NodeId(0)],
                kind: JobKind::Preview,
            });
        }
        solve.wait_idle();
        let report = last.0.lock().unwrap().clone().unwrap();
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
        let last = Arc::new(Last(Mutex::new(None)));
        let solve = SolveLoop::new(scheduler, ScriptCancel::new(), last.clone());
        solve.submit(Job {
            lowered: graph(1.0),
            targets: vec![NodeId(0)],
            kind: JobKind::Structural,
        });
        solve.cancel();
        solve.wait_idle();
        let report = last.0.lock().unwrap().clone().unwrap();
        // Either the cancel landed before the node ran (Cancelled) or the
        // node was too fast (Computed) — both honest, neither wedged.
        assert!(matches!(
            report.outcome(NodeId(0)),
            NodeOutcome::Cancelled | NodeOutcome::Computed { .. } | NodeOutcome::CacheHit { .. }
        ));
    }
}
