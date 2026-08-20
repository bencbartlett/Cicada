//! The latest-wins preview path (docs/12 §Solve generations, doc 39):
//! continuous param streams skip debounce entirely — each submission
//! **supersedes** whatever is pending, cancels the in-flight generation,
//! and each completed generation immediately starts the next with the
//! newest value. Supersession is nearly free because completed work landed
//! in the store — a slider drag is a stream of generations, each reusing
//! everything the previous one finished.
//!
//! Structural-edit debounce (~30 ms) is the caller's timer (the stage-5
//! server owns wall-clock policy); this type owns supersession only.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::cancel::CancelToken;
use crate::exec::{NoopObserver, Observer, Scheduler, SolveError, SolveReport};
use crate::graph::{NodeId, SolveGraph};

/// One preview request: a graph (typically the same topology with a new
/// param value) and the targets to pull.
pub struct PreviewJob {
    /// The graph to solve.
    pub graph: Arc<SolveGraph>,
    /// Pull targets.
    pub targets: Vec<NodeId>,
}

struct State {
    pending: Option<PreviewJob>,
    current: Option<CancelToken>,
    in_flight: bool,
    shutdown: bool,
    next_generation: u64,
    generations_run: u64,
    /// Last finished generation's report (cancelled or not).
    last: Option<Arc<SolveReport>>,
    /// Last generation that ran to completion uncancelled — what the
    /// viewer shows (docs/12: last COMPLETE value, no torn state).
    last_complete: Option<Arc<SolveReport>>,
    /// First engine-level error, if any (a store failure mid-preview).
    error: Option<SolveError>,
}

struct Shared {
    scheduler: Arc<Scheduler>,
    observer: Arc<dyn Observer>,
    state: Mutex<State>,
    wake: Condvar,
}

/// A worker running latest-wins preview generations.
pub struct PreviewSession {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewSession {
    /// Start a session on the given scheduler.
    #[must_use]
    pub fn new(scheduler: Arc<Scheduler>) -> Self {
        Self::with_observer(scheduler, Arc::new(NoopObserver))
    }

    /// Start with an observer receiving every generation's events.
    ///
    /// # Panics
    ///
    /// When the worker thread cannot spawn (process-level resource
    /// exhaustion) — a session without its worker would deadlock silently.
    #[must_use]
    pub fn with_observer(scheduler: Arc<Scheduler>, observer: Arc<dyn Observer>) -> Self {
        let shared = Arc::new(Shared {
            scheduler,
            observer,
            state: Mutex::new(State {
                pending: None,
                current: None,
                in_flight: false,
                shutdown: false,
                next_generation: 0,
                generations_run: 0,
                last: None,
                last_complete: None,
                error: None,
            }),
            wake: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = match std::thread::Builder::new()
            .name("cicada-preview".to_owned())
            .spawn(move || worker_loop(&worker_shared))
        {
            Ok(handle) => Some(handle),
            // A session without its worker would swallow every submit and
            // deadlock wait_idle — refuse loudly instead (thread spawn
            // failing means the process is already in serious trouble).
            Err(error) => panic!("preview worker thread could not spawn: {error}"),
        };
        Self { shared, worker }
    }

    /// Submit the newest state. Latest wins: any queued-but-unstarted job
    /// is replaced (its intermediate value will never be seen), and the
    /// in-flight generation is cancelled.
    pub fn submit(&self, job: PreviewJob) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.pending = Some(job);
        if let Some(token) = &state.current {
            token.cancel();
        }
        drop(state);
        self.shared.wake.notify_all();
    }

    /// Block until nothing is pending or in flight; returns the last
    /// complete (uncancelled) report, if any generation finished cleanly.
    #[must_use]
    pub fn wait_idle(&self) -> Option<Arc<SolveReport>> {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = self
            .shared
            .wake
            .wait_while(state, |state| state.in_flight || state.pending.is_some())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.last_complete.clone()
    }

    /// The last finished generation's report (cancelled generations
    /// included) — progress displays want this; viewers want
    /// [`Self::wait_idle`]'s complete one.
    #[must_use]
    pub fn last_report(&self) -> Option<Arc<SolveReport>> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last
            .clone()
    }

    /// Generations actually run (superseded-before-start jobs never count).
    #[must_use]
    pub fn generations_run(&self) -> u64 {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generations_run
    }

    /// The first engine-level error, if one occurred (taking it clears it).
    #[must_use]
    pub fn take_error(&self) -> Option<SolveError> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .error
            .take()
    }
}

impl Drop for PreviewSession {
    fn drop(&mut self) {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.shutdown = true;
            if let Some(token) = &state.current {
                token.cancel();
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
        // Wait for work (or shutdown), then take the NEWEST job.
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
            let generation = state.next_generation;
            state.next_generation += 1;
            (job, token, generation)
        };

        // catch_unwind: a panic anywhere inside solve (a user observer, an
        // executor invariant) re-throws on THIS thread via rayon. Without
        // the guard the worker would die between `in_flight = true` and
        // the bookkeeping below — wait_idle blocked forever, submits
        // silently swallowed, no error anywhere: the exact opposite of
        // fail-loudly. The panic becomes a surfaced SolveError instead.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.scheduler.solve(
                &job.graph,
                &job.targets,
                generation,
                &token,
                shared.observer.as_ref(),
            )
        }))
        .unwrap_or_else(|payload| {
            Err(SolveError::EnginePanic {
                message: crate::exec::panic_message(payload.as_ref()),
            })
        });

        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.in_flight = false;
            state.current = None;
            state.generations_run += 1;
            match result {
                Ok(report) => {
                    let report = Arc::new(report);
                    if !report.cancelled {
                        state.last_complete = Some(Arc::clone(&report));
                    }
                    state.last = Some(report);
                }
                Err(error) => {
                    if state.error.is_none() {
                        state.error = Some(error);
                    }
                }
            }
        }
        shared.wake.notify_all();
        // Loop: if a newer job arrived while solving, it starts
        // immediately — "each completed generation immediately starts the
        // next with the newest value".
    }
}
