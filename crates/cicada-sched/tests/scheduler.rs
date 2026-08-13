//! Stage-3 definition of done (doc 15): virtual-time fake-node tests
//! proving cache hits,
//! exact dirty cones, warm reopen computing nothing, cancel-to-idle
//! < 100 ms (virtual), and a synthetic 1,500-element map saturating a
//! 4-thread pool. Plus the failure-path contracts: red-with-element-IDs,
//! strict zip at run time, hole refusal, panic-to-red, blocked cones.
//!
//! No sleeps, no wall clock (doc 14): fake nodes advance a [`VirtualClock`],
//! and all cost/cancellation assertions are in virtual nanoseconds.

// Tests are exempt from the unwrap/expect denial, but the exemption only
// recognizes #[test] fns — not helpers in integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]
// Exact float `==` on values that pass through unchanged (exact-IEEE test
// carve-out, ledger revision 2026-08-12).
#![allow(clippy::float_cmp)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cicada_core::value::{HashedValue, List, ValueData};
use cicada_sched::{
    CancelToken, Clock as _, DiskStore, Event, Input, NodeDecl, NodeError, NodeFn, NodeId,
    NodeOutcome, Observer, PreviewJob, PreviewSession, Scheduler, SchedulerConfig, SolveGraph,
    VirtualClock,
};

// ------------------------------------------------------------- fakes --

/// Owned copies of executor events — the tests' oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    Started(String),
    CacheHit(String),
    ElementCacheHit(String, usize),
    Chunk { node: String, len: usize },
    Computed(String),
    Failed(String),
    Blocked { node: String, upstream: String },
    Cancelled(String),
}

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<Seen>>,
}

impl Observer for Recorder {
    fn on_event(&self, event: &Event<'_>) {
        let seen = match event {
            Event::NodeStarted { node } => Seen::Started((*node).to_owned()),
            Event::NodeCacheHit { node } => Seen::CacheHit((*node).to_owned()),
            Event::ElementCacheHit { node, index } => {
                Seen::ElementCacheHit((*node).to_owned(), *index)
            }
            Event::ChunkExecuted { node, len, .. } => Seen::Chunk {
                node: (*node).to_owned(),
                len: *len,
            },
            Event::NodeComputed { node, .. } => Seen::Computed((*node).to_owned()),
            Event::NodeFailed { node } => Seen::Failed((*node).to_owned()),
            Event::NodeBlocked { node, upstream } => Seen::Blocked {
                node: (*node).to_owned(),
                upstream: (*upstream).to_owned(),
            },
            Event::NodeCancelled { node } => Seen::Cancelled((*node).to_owned()),
        };
        self.events.lock().unwrap().push(seen);
    }
}

impl Recorder {
    fn computed(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|seen| match seen {
                Seen::Computed(node) => Some(node.clone()),
                _ => None,
            })
            .collect()
    }

    fn cache_hits(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|seen| match seen {
                Seen::CacheHit(node) => Some(node.clone()),
                _ => None,
            })
            .collect()
    }

    fn element_cache_hits(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| matches!(seen, Seen::ElementCacheHit(_, _)))
            .count()
    }

    fn chunk_lens(&self, node: &str) -> Vec<usize> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|seen| match seen {
                Seen::Chunk { node: n, len } if n == node => Some(*len),
                _ => None,
            })
            .collect()
    }
}

fn number(x: f64) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Number(x)).unwrap()
}

fn number_list(xs: &[f64], axis: Option<&str>) -> Arc<HashedValue> {
    HashedValue::new(ValueData::List(List {
        axis: axis.map(Arc::from),
        slots: xs.iter().map(|&x| Some(number(x))).collect(),
    }))
    .unwrap()
}

fn as_number(value: &HashedValue) -> f64 {
    let ValueData::Number(x) = value.data() else {
        panic!("expected Number, got {:?}", value.data().kind_name())
    };
    *x
}

/// A fake scalar op: advances virtual time by `cost`, then maps the sum of
/// its Number inputs through `f`.
fn fake_fn(
    clock: &Arc<VirtualClock>,
    cost: u64,
    f: impl Fn(f64) -> f64 + Send + Sync + 'static,
) -> NodeFn {
    let clock = Arc::clone(clock);
    Arc::new(move |inputs| {
        clock.advance(cost);
        let sum: f64 = inputs.iter().flatten().map(|value| as_number(value)).sum();
        Ok(vec![number(f(sum))])
    })
}

fn decl(name: &str, op: &str, inputs: Vec<Input>, fan: Vec<u8>, run: NodeFn) -> NodeDecl {
    NodeDecl {
        name: name.to_owned(),
        op: op.to_owned(),
        version: 1,
        body_hash: None,
        tolerance: None,
        inputs,
        fan,
        output_count: 1,
        run,
    }
}

fn port(node: usize) -> Input {
    Input::Port {
        node: NodeId(node),
        output: 0,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    clock: Arc<VirtualClock>,
    scheduler: Scheduler,
}

fn rig(threads: usize, tune: impl FnOnce(&mut SchedulerConfig)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let clock = Arc::new(VirtualClock::new());
    let mut config = SchedulerConfig {
        threads,
        ..SchedulerConfig::default()
    };
    tune(&mut config);
    let scheduler = Scheduler::new(Arc::new(store), Arc::clone(&clock) as _, config).unwrap();
    Rig {
        _dir: dir,
        clock,
        scheduler,
    }
}

/// The diamond: a → {b, c} → d, with `a` and `b` each fed by a param value.
/// Costs are per-node virtual nanos.
fn diamond(clock: &Arc<VirtualClock>, a_param: f64, b_param: f64) -> SolveGraph {
    SolveGraph::new(vec![
        decl(
            "a",
            "fake.a",
            vec![Input::Value(number(a_param))],
            vec![0],
            fake_fn(clock, 1_000, |x| x + 1.0),
        ),
        decl(
            "b",
            "fake.b",
            vec![port(0), Input::Value(number(b_param))],
            vec![0, 0],
            fake_fn(clock, 1_000, |x| x * 2.0),
        ),
        decl(
            "c",
            "fake.c",
            vec![port(0)],
            vec![0],
            fake_fn(clock, 1_000, |x| x * 3.0),
        ),
        decl(
            "d",
            "fake.d",
            vec![port(1), port(2)],
            vec![0, 0],
            fake_fn(clock, 1_000, |x| x + 0.5),
        ),
    ])
    .unwrap()
}

fn solve(
    scheduler: &Scheduler,
    graph: &SolveGraph,
    targets: &[NodeId],
    recorder: &Recorder,
) -> cicada_sched::SolveReport {
    scheduler
        .solve(graph, targets, 0, &CancelToken::new(), recorder)
        .unwrap()
}

// ---------------------------------------------------------- cache hits --

#[test]
fn second_solve_hits_cache_and_computes_nothing() {
    let rig = rig(2, |_| {});
    let graph = diamond(&rig.clock, 1.0, 2.0);
    let first = Recorder::default();
    solve(&rig.scheduler, &graph, &[NodeId(3)], &first);
    assert_eq!(first.computed().len(), 4, "cold solve computes all four");

    let second = Recorder::default();
    let report = solve(&rig.scheduler, &graph, &[NodeId(3)], &second);
    assert_eq!(
        second.computed(),
        Vec::<String>::new(),
        "warm solve computes NOTHING"
    );
    assert_eq!(second.cache_hits().len(), 4);
    for index in 0..4 {
        assert!(
            matches!(report.outcome(NodeId(index)), NodeOutcome::CacheHit { .. }),
            "node {index} must be a cache hit"
        );
    }
}

#[test]
fn results_are_correct_and_deterministic_across_thread_counts() {
    let expected = {
        let rig = rig(1, |_| {});
        let graph = diamond(&rig.clock, 1.0, 2.0);
        let report = solve(&rig.scheduler, &graph, &[NodeId(3)], &Recorder::default());
        // a=2, b=(2+2)*2=8, c=6, d=(8+6)+0.5=14.5
        let hash = report.outcome(NodeId(3)).output_hashes().unwrap()[0];
        let value = rig.scheduler.store().load_value(&hash).unwrap();
        assert_eq!(as_number(&value), 14.5);
        hash
    };
    let parallel = {
        let rig = rig(4, |_| {});
        let graph = diamond(&rig.clock, 1.0, 2.0);
        let report = solve(&rig.scheduler, &graph, &[NodeId(3)], &Recorder::default());
        report.outcome(NodeId(3)).output_hashes().unwrap()[0]
    };
    assert_eq!(expected, parallel, "same bytes regardless of parallelism");
}

// ---------------------------------------------------------- dirty cone --

#[test]
fn dirty_cone_is_exact_on_param_edit() {
    let rig = rig(2, |_| {});
    let graph = diamond(&rig.clock, 1.0, 2.0);
    solve(&rig.scheduler, &graph, &[NodeId(3)], &Recorder::default());

    // Edit b's param: the dirty set is b's downstream cone {b, d} — a and
    // c are untouched by construction (docs/12 §Solve generations).
    let edited = diamond(&rig.clock, 1.0, 9.0);
    let recorder = Recorder::default();
    solve(&rig.scheduler, &edited, &[NodeId(3)], &recorder);

    let cone_mask = edited.downstream_cone(&[NodeId(1)]);
    let expected: Vec<String> = edited
        .nodes()
        .iter()
        .enumerate()
        .filter(|&(index, _)| cone_mask[index])
        .map(|(_, node)| node.name.clone())
        .collect();
    let mut computed = recorder.computed();
    computed.sort();
    let mut expected_sorted = expected;
    expected_sorted.sort();
    assert_eq!(
        computed, expected_sorted,
        "recompute set == exact dirty cone"
    );
    assert_eq!(recorder.cache_hits().len(), 2, "a and c hit cache");
}

#[test]
fn early_cutoff_stops_downstream_when_bytes_repeat() {
    let rig = rig(2, |_| {});
    let clamp = |x: f64| x.min(10.0);
    let graph = |param: f64| {
        SolveGraph::new(vec![
            decl(
                "clamped",
                "fake.clamp",
                vec![Input::Value(number(param))],
                vec![0],
                fake_fn(&rig.clock, 1_000, clamp),
            ),
            decl(
                "after",
                "fake.after",
                vec![port(0)],
                vec![0],
                fake_fn(&rig.clock, 1_000, |x| x * 2.0),
            ),
        ])
        .unwrap()
    };
    solve(
        &rig.scheduler,
        &graph(12.0),
        &[NodeId(1)],
        &Recorder::default(),
    );

    // 12 → 15: `clamped` recomputes (its key changed) but produces the
    // same bytes (10.0); `after`'s key is therefore unchanged — cache hit
    // with no special same-value detection anywhere (docs/12).
    let recorder = Recorder::default();
    solve(&rig.scheduler, &graph(15.0), &[NodeId(1)], &recorder);
    assert_eq!(recorder.computed(), vec!["clamped".to_owned()]);
    assert_eq!(recorder.cache_hits(), vec!["after".to_owned()]);
}

// ---------------------------------------------------------- warm reopen --

#[test]
fn warm_reopen_computes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(VirtualClock::new());
    {
        let (store, report) = DiskStore::open(dir.path()).unwrap();
        assert_eq!(report.memo_entries, 0);
        let scheduler = Scheduler::new(
            Arc::new(store),
            Arc::clone(&clock) as _,
            SchedulerConfig {
                threads: 2,
                ..SchedulerConfig::default()
            },
        )
        .unwrap();
        let graph = diamond(&clock, 1.0, 2.0);
        solve(&scheduler, &graph, &[NodeId(3)], &Recorder::default());
    } // scheduler and store dropped — "the app closed"

    let (store, report) = DiskStore::open(dir.path()).unwrap();
    assert!(report.memo_entries >= 4, "memo log replayed");
    assert_eq!(report.recovery, None);
    let scheduler = Scheduler::new(
        Arc::new(store),
        Arc::clone(&clock) as _,
        SchedulerConfig {
            threads: 2,
            ..SchedulerConfig::default()
        },
    )
    .unwrap();
    let graph = diamond(&clock, 1.0, 2.0);
    let recorder = Recorder::default();
    let before = clock.now_nanos();
    let report = solve(&scheduler, &graph, &[NodeId(3)], &recorder);
    assert_eq!(
        recorder.computed(),
        Vec::<String>::new(),
        "warm reopen computes NOTHING"
    );
    assert_eq!(clock.now_nanos(), before, "zero virtual work done");
    let hash = report.outcome(NodeId(3)).output_hashes().unwrap()[0];
    let value = scheduler.store().load_value(&hash).unwrap();
    assert_eq!(as_number(&value), 14.5, "values rehydrate from the store");
}

#[test]
fn torn_log_tail_is_recovered_truncated_and_healed() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(VirtualClock::new());
    let scheduler_at = |dir: &std::path::Path| {
        let (store, report) = DiskStore::open(dir).unwrap();
        let scheduler = Scheduler::new(
            Arc::new(store),
            Arc::clone(&clock) as _,
            SchedulerConfig {
                threads: 1,
                ..SchedulerConfig::default()
            },
        )
        .unwrap();
        (scheduler, report)
    };
    {
        let (scheduler, _) = scheduler_at(dir.path());
        let graph = diamond(&clock, 1.0, 2.0);
        solve(&scheduler, &graph, &[NodeId(3)], &Recorder::default());
    }
    // Simulate a crash mid-append: a torn frame at the tail.
    let log = dir.path().join("memo.log");
    let clean_len = std::fs::metadata(&log).unwrap().len();
    let mut bytes = std::fs::read(&log).unwrap();
    bytes.extend_from_slice(&[200, 0, 0, 0, 1, 2, 3]); // claims 200 bytes, has 3
    std::fs::write(&log, bytes).unwrap();

    // Session 2: tear reported AND the log truncated back to the intact
    // prefix, so records appended NOW stay replayable (regression:
    // adversarial review — an untruncated tear made the log a permanent
    // write-only black hole).
    {
        let (scheduler, report) = scheduler_at(dir.path());
        assert_eq!(report.recovery, Some(cicada_sched::LogRecovery::TornTail));
        assert!(report.memo_entries >= 4, "entries before the tear intact");
        assert_eq!(
            std::fs::metadata(&log).unwrap().len(),
            clean_len,
            "torn bytes truncated before reuse"
        );
        // New work in the post-recovery session…
        let graph = diamond(&clock, 5.0, 6.0);
        solve(&scheduler, &graph, &[NodeId(3)], &Recorder::default());
    }
    // …replays in session 3: nothing recomputes, no damage reported.
    {
        let (scheduler, report) = scheduler_at(dir.path());
        assert_eq!(report.recovery, None, "healed log opens clean");
        let graph = diamond(&clock, 5.0, 6.0);
        let recorder = Recorder::default();
        solve(&scheduler, &graph, &[NodeId(3)], &recorder);
        assert_eq!(
            recorder.computed(),
            Vec::<String>::new(),
            "post-recovery records survived the reopen"
        );
    }
}

#[test]
fn corrupt_mid_log_record_is_reported_with_counts_and_healed() {
    let dir = tempfile::tempdir().unwrap();
    {
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        // Two memo records with a known boundary between them.
        let key_a = cicada_sched::node_key(&cicada_sched::KeyInputs {
            op: "probe.a",
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: &[],
            fan: &[],
        });
        let key_b = cicada_sched::node_key(&cicada_sched::KeyInputs {
            op: "probe.b",
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: &[],
            fan: &[],
        });
        store.record_memo(key_a, &[]).unwrap();
        store.record_memo(key_b, &[]).unwrap();
    }
    let log = dir.path().join("memo.log");
    let mut bytes = std::fs::read(&log).unwrap();
    // Corrupt the FIRST record's body (postcard enum discriminant out of
    // range) — everything after must be reported dropped, with counts,
    // never mislabeled a "torn tail".
    let total = bytes.len();
    bytes[4] = 0xFF;
    std::fs::write(&log, bytes).unwrap();

    let (_store, report) = DiskStore::open(dir.path()).unwrap();
    assert_eq!(
        report.recovery,
        Some(cicada_sched::LogRecovery::CorruptRecord {
            offset: 0,
            bytes_dropped: total,
        }),
        "mid-file corruption distinguished from a tail tear, with counts"
    );
    assert_eq!(report.memo_entries, 0);
    // Truncated at the damage: the next open is clean.
    let (_store, report) = DiskStore::open(dir.path()).unwrap();
    assert_eq!(report.recovery, None);
}

// --------------------------------------------------------- cancellation --

#[test]
fn cancel_to_idle_under_100_virtual_ms() {
    // 200 elements × 10 ms each; chunks of 2 (20 ms of work per chunk) on
    // 4 threads. Worst case after cancel: the ≤4 in-flight chunks finish —
    // ≤ 80 ms of virtual work, under the 100 ms budget (doc 15 DoD).
    let rig = rig(4, |config| {
        config.chunk_target_nanos = 20_000_000;
        config.cold_element_nanos = 10_000_000;
    });
    let token = CancelToken::new();
    let cancel_at = Arc::new(AtomicU64::new(0));

    let element_cost = 10_000_000_u64;
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        let token = token.clone();
        let cancel_at = Arc::clone(&cancel_at);
        Arc::new(move |inputs| {
            clock.advance(element_cost);
            let x = as_number(inputs[0].as_ref().unwrap());
            // Deterministic trigger: element #42 pulls the plug.
            if (x - 42.0).abs() < 0.5 && !token.is_cancelled() {
                cancel_at.store(clock.now_nanos(), Ordering::SeqCst);
                token.cancel();
            }
            Ok(vec![number(x)])
        })
    };
    let elements: Vec<f64> = (0..200).map(f64::from).collect();
    let graph = SolveGraph::new(vec![decl(
        "mapped",
        "fake.map",
        vec![Input::Value(number_list(&elements, None))],
        vec![1],
        run,
    )])
    .unwrap();

    let report = rig
        .scheduler
        .solve(&graph, &[NodeId(0)], 0, &token, &Recorder::default())
        .unwrap();
    // solve() returning IS idle — no work continues after it.
    assert!(report.cancelled);
    assert!(
        matches!(report.outcome(NodeId(0)), NodeOutcome::Cancelled),
        "cancelled mid-fan yields no output"
    );
    let cancelled_at = cancel_at.load(Ordering::SeqCst);
    assert!(cancelled_at > 0, "the trigger fired");
    let after = rig.clock.now_nanos().saturating_sub(cancelled_at);
    assert!(
        after < 100_000_000,
        "cancel-to-idle did {after} ns of virtual work (budget 100 ms)"
    );
}

// ----------------------------------------------------------- saturation --

/// A barrier with a loud timeout: if the scheduler ever fails to run
/// `size` chunks concurrently, the test fails in 30 s instead of hanging.
struct TimeoutBarrier {
    size: usize,
    state: Mutex<(usize, u64)>,
    signal: Condvar,
}

impl TimeoutBarrier {
    fn new(size: usize) -> Self {
        Self {
            size,
            state: Mutex::new((0, 0)),
            signal: Condvar::new(),
        }
    }

    fn arrive(&self) {
        let mut state = self.state.lock().unwrap();
        let generation = state.1;
        state.0 += 1;
        if state.0 == self.size {
            state.0 = 0;
            state.1 += 1;
            drop(state);
            self.signal.notify_all();
            return;
        }
        let (state, timeout) = self
            .signal
            .wait_timeout_while(state, Duration::from_secs(30), |state| {
                state.1 == generation
            })
            .unwrap();
        drop(state);
        assert!(
            !timeout.timed_out(),
            "scheduler failed to run {} chunks concurrently — cores not saturated",
            self.size
        );
    }
}

#[test]
fn synthetic_1500_element_map_saturates_a_4_thread_pool() {
    let threads = 4;
    // 1,500 elements in chunks of 125 → 12 chunks → 3 waves of exactly 4
    // concurrent chunks. Every element waits on a 4-way barrier, so the
    // solve COMPLETES only if 4 chunks genuinely run at once, and the
    // high-water mark proves it.
    let rig = rig(threads, |config| {
        config.chunk_target_nanos = 125_000_000;
        config.cold_element_nanos = 1_000_000;
    });
    let barrier = Arc::new(TimeoutBarrier::new(threads));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let high_water = Arc::new(AtomicUsize::new(0));

    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        let barrier = Arc::clone(&barrier);
        let in_flight = Arc::clone(&in_flight);
        let high_water = Arc::clone(&high_water);
        Arc::new(move |inputs| {
            let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            high_water.fetch_max(current, Ordering::SeqCst);
            barrier.arrive();
            in_flight.fetch_sub(1, Ordering::SeqCst);
            clock.advance(1_000_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            Ok(vec![number(x * 2.0)])
        })
    };
    let elements: Vec<f64> = (0..1_500).map(f64::from).collect();
    let graph = SolveGraph::new(vec![decl(
        "mapped",
        "fake.map",
        vec![Input::Value(number_list(&elements, None))],
        vec![1],
        run,
    )])
    .unwrap();

    let recorder = Recorder::default();
    let report = solve(&rig.scheduler, &graph, &[NodeId(0)], &recorder);
    assert!(
        matches!(
            report.outcome(NodeId(0)),
            NodeOutcome::Computed {
                elements: 1_500,
                ..
            }
        ),
        "all 1,500 elements computed"
    );
    assert_eq!(
        high_water.load(Ordering::SeqCst),
        threads,
        "all {threads} workers ran elements simultaneously"
    );
    let lens = recorder.chunk_lens("mapped");
    assert_eq!(lens.len(), 12, "cost-sized chunks: 1500 elements / 125");
    assert!(lens.iter().all(|&len| len == 125));
    // The output is index-addressed regardless of chunk timing.
    let hash = report.outcome(NodeId(0)).output_hashes().unwrap()[0];
    let value = rig.scheduler.store().load_value(&hash).unwrap();
    let ValueData::List(list) = value.data() else {
        panic!("fan output is a list")
    };
    assert_eq!(as_number(list.slots[1_499].as_ref().unwrap()), 2_998.0);
}

// --------------------------------------------------- latest-wins preview --

#[test]
fn preview_stream_is_latest_wins_and_lands_on_the_newest_value() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let clock = Arc::new(VirtualClock::new());
    let scheduler = Arc::new(
        Scheduler::new(
            Arc::new(store),
            Arc::clone(&clock) as _,
            SchedulerConfig {
                threads: 2,
                ..SchedulerConfig::default()
            },
        )
        .unwrap(),
    );

    let graph_for = |param: f64| {
        Arc::new(
            SolveGraph::new(vec![
                decl(
                    "base",
                    "fake.base",
                    vec![Input::Value(number(param))],
                    vec![0],
                    fake_fn(&clock, 5_000_000, |x| x + 1.0),
                ),
                decl(
                    "out",
                    "fake.out",
                    vec![port(0)],
                    vec![0],
                    fake_fn(&clock, 5_000_000, |x| x * 10.0),
                ),
            ])
            .unwrap(),
        )
    };

    let session = PreviewSession::new(Arc::clone(&scheduler));
    for value in 1..=20 {
        session.submit(PreviewJob {
            graph: graph_for(f64::from(value)),
            targets: vec![NodeId(1)],
        });
    }
    let last = session.wait_idle().expect("a complete generation exists");
    assert!(!last.cancelled);
    let hash = last.outcome(NodeId(1)).output_hashes().unwrap()[0];
    let value = scheduler.store().load_value(&hash).unwrap();
    assert_eq!(
        as_number(&value),
        210.0,
        "final state is the NEWEST value (20+1)*10"
    );
    assert!(session.take_error().is_none());
    let generations = session.generations_run();
    assert!(
        (1..=20).contains(&generations),
        "supersession collapses the stream ({generations} generations for 20 submissions)"
    );
    drop(session);

    // Whatever generations completed live in the cache: re-solving the
    // final graph computes nothing.
    let recorder = Recorder::default();
    let report = scheduler
        .solve(
            &graph_for(20.0),
            &[NodeId(1)],
            99,
            &CancelToken::new(),
            &recorder,
        )
        .unwrap();
    assert_eq!(recorder.computed(), Vec::<String>::new());
    assert!(matches!(
        report.outcome(NodeId(1)),
        NodeOutcome::CacheHit { .. }
    ));
}

// ------------------------------------------------------- element caching --

#[test]
fn expensive_elements_memoize_individually_and_dedupe_across_lists() {
    // Threshold 1 ns: any measured cost enables element caching from the
    // second solve on (the first solve calibrates).
    let rig = rig(2, |config| {
        config.element_cache_min_nanos = 1;
    });
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        Arc::new(move |inputs| {
            clock.advance(1_000_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            Ok(vec![number(x * 2.0)])
        })
    };
    let graph_for = |xs: &[f64]| {
        SolveGraph::new(vec![decl(
            "mapped",
            "fake.expensive",
            vec![Input::Value(number_list(xs, None))],
            vec![1],
            Arc::clone(&run),
        )])
        .unwrap()
    };

    // Solve 1 (cold — no stats, element cache off, calibrates).
    solve(
        &rig.scheduler,
        &graph_for(&[1.0, 2.0, 3.0]),
        &[NodeId(0)],
        &Recorder::default(),
    );
    // Solve 2: different list, no overlap benefit yet (solve 1 stored no
    // element entries), but THIS solve stores them.
    let second = Recorder::default();
    solve(
        &rig.scheduler,
        &graph_for(&[2.0, 3.0, 4.0]),
        &[NodeId(0)],
        &second,
    );
    assert_eq!(second.element_cache_hits(), 0);
    // Solve 3: overlaps solve 2 on {3, 4} → two element-level hits.
    let third = Recorder::default();
    solve(
        &rig.scheduler,
        &graph_for(&[3.0, 4.0, 5.0]),
        &[NodeId(0)],
        &third,
    );
    assert_eq!(
        third.element_cache_hits(),
        2,
        "elements shared with the previous list answer from the element memo"
    );
}

#[test]
fn chunk_sizing_follows_measured_cost() {
    // 10 ms elements, 25 ms target → chunks of 2 once samples exist.
    // One worker: the shared VIRTUAL clock would let concurrent chunks
    // inflate each other's elapsed measurement (per-thread wall clocks in
    // production do not) — sequential chunks keep the samples exact.
    let rig = rig(1, |config| {
        config.chunk_target_nanos = 25_000_000;
        config.cold_element_nanos = 1_000_000;
    });
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        Arc::new(move |inputs| {
            clock.advance(10_000_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            Ok(vec![number(x)])
        })
    };
    let graph_for = |offset: f64| {
        let xs: Vec<f64> = (0..8).map(|i| f64::from(i) + offset).collect();
        SolveGraph::new(vec![decl(
            "mapped",
            "fake.tenms",
            vec![Input::Value(number_list(&xs, None))],
            vec![1],
            Arc::clone(&run),
        )])
        .unwrap()
    };
    // Cold: 1 ms assumed → 25 elements by cost, capped at n/workers = 8.
    let cold = Recorder::default();
    solve(&rig.scheduler, &graph_for(0.0), &[NodeId(0)], &cold);
    assert_eq!(cold.chunk_lens("mapped"), vec![8]);
    // Warm estimate 10 ms → 25/10 = chunks of 2.
    let warm = Recorder::default();
    solve(&rig.scheduler, &graph_for(100.0), &[NodeId(0)], &warm);
    assert_eq!(warm.chunk_lens("mapped"), vec![2, 2, 2, 2]);
}

// ------------------------------------------------------------- failures --

#[test]
fn element_failures_are_red_with_ids_and_block_downstream() {
    let rig = rig(2, |_| {});
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        Arc::new(move |inputs| {
            clock.advance(1_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            if (x as u64) % 2 == 1 {
                return Err(NodeError::new(format!("odd element {x} refused")));
            }
            Ok(vec![number(x)])
        })
    };
    let graph = SolveGraph::new(vec![
        decl(
            "mapped",
            "fake.odd",
            vec![Input::Value(number_list(
                &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
                None,
            ))],
            vec![1],
            run,
        ),
        decl(
            "after",
            "fake.after2",
            vec![port(0)],
            vec![0],
            fake_fn(&rig.clock, 1_000, |x| x),
        ),
    ])
    .unwrap();
    let report = solve(&rig.scheduler, &graph, &[NodeId(1)], &Recorder::default());
    let NodeOutcome::Failed(failure) = report.outcome(NodeId(0)) else {
        panic!("mapped must be red")
    };
    assert_eq!(
        failure.element_ids,
        vec![1, 3, 5],
        "ALL offending element IDs"
    );
    assert!(failure.message.contains("element 1"), "{}", failure.message);
    assert!(
        failure.message.contains("and 2 more elements"),
        "{}",
        failure.message
    );
    let NodeOutcome::Blocked { upstream } = report.outcome(NodeId(1)) else {
        panic!("downstream of red must be blocked, not run")
    };
    assert_eq!(upstream, "mapped");
    assert_eq!(report.failures().len(), 1);
}

#[test]
fn strict_zip_mismatch_fails_with_counts() {
    let rig = rig(2, |_| {});
    let graph = SolveGraph::new(vec![decl(
        "zipped",
        "fake.zip",
        vec![
            Input::Value(number_list(&[1.0, 2.0, 3.0], None)),
            Input::Value(number_list(&[1.0, 2.0, 3.0, 4.0, 5.0], None)),
        ],
        vec![1, 1],
        fake_fn(&rig.clock, 1_000, |x| x),
    )])
    .unwrap();
    let report = solve(&rig.scheduler, &graph, &[NodeId(0)], &Recorder::default());
    let NodeOutcome::Failed(failure) = report.outcome(NodeId(0)) else {
        panic!("zip mismatch must be red")
    };
    assert!(
        failure.message.contains("zip is strict: 3 vs 5"),
        "counts in the error: {}",
        failure.message
    );
}

#[test]
fn holes_in_fanned_input_are_refused_with_ids() {
    let rig = rig(2, |_| {});
    let holed = HashedValue::new(ValueData::List(List {
        axis: None,
        slots: vec![Some(number(1.0)), None, Some(number(3.0))],
    }))
    .unwrap();
    let graph = SolveGraph::new(vec![decl(
        "mapped",
        "fake.holed",
        vec![Input::Value(holed)],
        vec![1],
        fake_fn(&rig.clock, 1_000, |x| x),
    )])
    .unwrap();
    let report = solve(&rig.scheduler, &graph, &[NodeId(0)], &Recorder::default());
    let NodeOutcome::Failed(failure) = report.outcome(NodeId(0)) else {
        panic!("holes must refuse")
    };
    assert_eq!(failure.element_ids, vec![1]);
    assert!(failure.message.contains("compact"), "{}", failure.message);
}

#[test]
fn node_panic_becomes_a_red_node_not_a_crash() {
    let rig = rig(2, |_| {});
    let run: NodeFn = Arc::new(move |_inputs| panic!("kaboom: count must be >= 0"));
    let graph = SolveGraph::new(vec![
        decl(
            "boom",
            "fake.boom",
            vec![Input::Value(number(1.0))],
            vec![0],
            run,
        ),
        decl(
            "after",
            "fake.after3",
            vec![port(0)],
            vec![0],
            fake_fn(&rig.clock, 1_000, |x| x),
        ),
        decl(
            "independent",
            "fake.indep",
            vec![Input::Value(number(7.0))],
            vec![0],
            fake_fn(&rig.clock, 1_000, |x| x + 1.0),
        ),
    ])
    .unwrap();
    let report = solve(
        &rig.scheduler,
        &graph,
        &[NodeId(1), NodeId(2)],
        &Recorder::default(),
    );
    let NodeOutcome::Failed(failure) = report.outcome(NodeId(0)) else {
        panic!("panic must become a red node")
    };
    assert!(failure.message.contains("kaboom"), "{}", failure.message);
    assert!(matches!(
        report.outcome(NodeId(1)),
        NodeOutcome::Blocked { .. }
    ));
    assert!(
        matches!(report.outcome(NodeId(2)), NodeOutcome::Computed { .. }),
        "an independent branch still solves — red cones are scoped"
    );
}

// ------------------------------------------------------ fan-out shapes --

#[test]
fn fanned_multi_output_assembles_per_port_lists_and_keeps_axis() {
    let rig = rig(2, |_| {});
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        Arc::new(move |inputs| {
            clock.advance(1_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            Ok(vec![number(x * 2.0), number(x * 3.0)])
        })
    };
    let mut node = decl(
        "split",
        "fake.split",
        vec![Input::Value(number_list(&[1.0, 2.0], Some("parts")))],
        vec![1],
        run,
    );
    node.output_count = 2;
    let graph = SolveGraph::new(vec![node]).unwrap();
    let report = solve(&rig.scheduler, &graph, &[NodeId(0)], &Recorder::default());
    let hashes = report.outcome(NodeId(0)).output_hashes().unwrap().to_vec();
    assert_eq!(hashes.len(), 2);
    for (port, factor) in [(0, 2.0), (1, 3.0)] {
        let value = rig.scheduler.store().load_value(&hashes[port]).unwrap();
        let ValueData::List(list) = value.data() else {
            panic!("fan output is a list")
        };
        assert_eq!(list.axis.as_deref(), Some("parts"), "axis survives the map");
        assert_eq!(as_number(list.slots[1].as_ref().unwrap()), 2.0 * factor);
    }
}

// ------------------------------------- review regressions (stage 3) --

// A memo record whose arity disagrees with the node (corrupt, stale, or
// foreign) used to index out of bounds — a bare panic out of solve(). It
// must instead be tombstoned and recomputed: the cache never owes
// correctness.
#[test]
fn wrong_arity_memo_record_heals_by_recompute_not_panic() {
    let rig = rig(2, |_| {});
    let graph = SolveGraph::new(vec![
        decl(
            "up",
            "fake.up",
            vec![Input::Value(number(3.0))],
            vec![0],
            fake_fn(&rig.clock, 1_000, |x| x + 1.0),
        ),
        decl(
            "down",
            "fake.down",
            vec![port(0)],
            vec![0],
            fake_fn(&rig.clock, 1_000, |x| x * 2.0),
        ),
    ])
    .unwrap();
    // Plant a ZERO-output memo record at `up`'s real key.
    let bad_key = cicada_sched::node_key(&cicada_sched::KeyInputs {
        op: "fake.up",
        version: 1,
        body_hash: None,
        tolerance: None,
        inputs: &[Some(number(3.0).hash())],
        fan: &[0],
    });
    rig.scheduler.store().record_memo(bad_key, &[]).unwrap();

    let report = solve(&rig.scheduler, &graph, &[NodeId(1)], &Recorder::default());
    assert!(
        matches!(report.outcome(NodeId(0)), NodeOutcome::Computed { .. }),
        "bad record tombstoned and recomputed, not trusted: {:?}",
        report.outcome(NodeId(0))
    );
    let hash = report.outcome(NodeId(1)).output_hashes().unwrap()[0];
    assert_eq!(
        as_number(&rig.scheduler.store().load_value(&hash).unwrap()),
        8.0
    );
    // The healed entry answers the next solve.
    let recorder = Recorder::default();
    solve(&rig.scheduler, &graph, &[NodeId(1)], &recorder);
    assert_eq!(recorder.computed(), Vec::<String>::new());
}

// A panic anywhere inside solve (here: a user observer) used to kill the
// preview worker between `in_flight = true` and the bookkeeping —
// wait_idle blocked forever, submits silently swallowed, no error
// anywhere. It must surface as a SolveError and leave the session alive.
#[test]
fn preview_survives_a_panicking_observer_and_surfaces_the_panic() {
    struct PanickingObserver;
    impl Observer for PanickingObserver {
        fn on_event(&self, event: &Event<'_>) {
            if matches!(event, Event::NodeComputed { .. }) {
                panic!("probe: observer bug");
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let clock = Arc::new(VirtualClock::new());
    let scheduler = Arc::new(
        Scheduler::new(
            Arc::new(store),
            Arc::clone(&clock) as _,
            SchedulerConfig {
                threads: 2,
                ..SchedulerConfig::default()
            },
        )
        .unwrap(),
    );
    let graph = Arc::new(
        SolveGraph::new(vec![decl(
            "boomwatch",
            "fake.boomwatch",
            vec![Input::Value(number(1.0))],
            vec![0],
            fake_fn(&clock, 1_000, |x| x),
        )])
        .unwrap(),
    );
    let session =
        PreviewSession::with_observer(Arc::clone(&scheduler), Arc::new(PanickingObserver));
    session.submit(PreviewJob {
        graph: Arc::clone(&graph),
        targets: vec![NodeId(0)],
    });
    // wait_idle RETURNS (no wedge)…
    let complete = session.wait_idle();
    assert!(
        complete.is_none(),
        "the panicked generation never completed"
    );
    // …and the panic is surfaced loudly, not swallowed.
    let error = session.take_error().expect("panic surfaced as an error");
    assert!(
        error.to_string().contains("observer bug"),
        "payload text kept: {error}"
    );
    // The worker is still alive: a second submit runs (and panics again —
    // the session keeps reporting rather than wedging).
    session.submit(PreviewJob {
        graph: Arc::clone(&graph),
        targets: vec![NodeId(0)],
    });
    let _ = session.wait_idle();
    assert!(session.generations_run() >= 2, "worker survived the panic");
}

// A memo entry whose blob was corrupted on disk: the first solve fails
// LOUDLY (typed error) and tombstones the entry + quarantines the blob;
// the next solve recomputes cleanly — never a permanent wedge.
#[test]
fn broken_memo_promise_heals_across_solves() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(VirtualClock::new());
    let scheduler_at = || {
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        Scheduler::new(
            Arc::new(store),
            Arc::clone(&clock) as _,
            SchedulerConfig {
                threads: 1,
                ..SchedulerConfig::default()
            },
        )
        .unwrap()
    };
    // `down` takes a param so a solve can make DOWN miss (forcing it to
    // hydrate `up`'s value) while `up` itself cache-hits hash-only.
    let graph = |param: f64| {
        SolveGraph::new(vec![
            decl(
                "up",
                "fake.uph",
                vec![Input::Value(number(3.0))],
                vec![0],
                fake_fn(&clock, 1_000, |x| x + 1.0),
            ),
            decl(
                "down",
                "fake.downh",
                vec![port(0), Input::Value(number(param))],
                vec![0, 0],
                fake_fn(&clock, 1_000, |x| x * 2.0),
            ),
        ])
        .unwrap()
    };
    let up_hash = {
        let scheduler = scheduler_at();
        let report = solve(&scheduler, &graph(1.0), &[NodeId(1)], &Recorder::default());
        report.outcome(NodeId(0)).output_hashes().unwrap()[0]
    };
    // Corrupt `up`'s blob on disk.
    let hex = up_hash.to_hex();
    let blob = dir
        .path()
        .join("values")
        .join(&hex[..2])
        .join(format!("{hex}.zst"));
    std::fs::write(&blob, b"garbage").unwrap();

    // Solve 2 (fresh store, nothing in memory; down's param changed so it
    // MISSES): up cache-hits hash-only, down computes and needs up's
    // value → loud typed failure, entry tombstoned.
    {
        let scheduler = scheduler_at();
        let error = scheduler
            .solve(
                &graph(2.0),
                &[NodeId(1)],
                0,
                &CancelToken::new(),
                &Recorder::default(),
            )
            .expect_err("broken promise fails loudly");
        assert!(
            matches!(
                error,
                cicada_sched::SolveError::Store(
                    cicada_sched::StoreError::Decode { .. }
                        | cicada_sched::StoreError::CorruptValue { .. }
                        | cicada_sched::StoreError::MissingValue { .. }
                )
            ),
            "typed store error, got {error}"
        );
    }
    // Solve 3: the tombstone forces recompute; everything is whole again.
    {
        let scheduler = scheduler_at();
        let recorder = Recorder::default();
        let report = solve(&scheduler, &graph(2.0), &[NodeId(1)], &recorder);
        assert!(recorder.computed().contains(&"up".to_owned()), "recomputed");
        let hash = report.outcome(NodeId(1)).output_hashes().unwrap()[0];
        assert_eq!(
            as_number(&scheduler.store().load_value(&hash).unwrap()),
            12.0,
            "(3+1 + 2) * 2 — whole again"
        );
    }
}

// Cost samples must count only elements that actually EXECUTED — counting
// element-cache hits diluted the per-element estimate toward zero with
// warm use, silently growing chunks past the cancellation budget.
#[test]
fn cost_samples_exclude_element_cache_hits() {
    let rig = rig(1, |config| {
        config.element_cache_min_nanos = 1;
    });
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        Arc::new(move |inputs| {
            clock.advance(1_000_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            Ok(vec![number(x * 2.0)])
        })
    };
    let graph_for = |xs: &[f64]| {
        SolveGraph::new(vec![decl(
            "mapped",
            "fake.pure_cost",
            vec![Input::Value(number_list(xs, None))],
            vec![1],
            Arc::clone(&run),
        )])
        .unwrap()
    };
    // Calibrate (all computed), then warm solves with heavy overlap.
    solve(
        &rig.scheduler,
        &graph_for(&[1.0, 2.0, 3.0]),
        &[NodeId(0)],
        &Recorder::default(),
    );
    solve(
        &rig.scheduler,
        &graph_for(&[2.0, 3.0, 4.0]),
        &[NodeId(0)],
        &Recorder::default(),
    );
    solve(
        &rig.scheduler,
        &graph_for(&[3.0, 4.0, 5.0]),
        &[NodeId(0)],
        &Recorder::default(),
    );
    let stats = rig.scheduler.store().stats("fake.pure_cost").unwrap();
    assert_eq!(
        stats.per_element_nanos(),
        Some(1_000_000),
        "estimate stays the TRUE per-element cost under warm overlap: {stats:?}"
    );
}

// A cancelled COLD fan must still record its partial cost sample —
// otherwise a preview stream superseding a cold expensive map never
// calibrates, element caching never enables, and no generation ever makes
// forward progress.
#[test]
fn cancelled_cold_fan_still_calibrates() {
    let rig = rig(1, |config| {
        config.chunk_target_nanos = 20_000_000;
        config.cold_element_nanos = 10_000_000;
        config.element_cache_min_nanos = 1;
    });
    let token = CancelToken::new();
    let run: NodeFn = {
        let clock = Arc::clone(&rig.clock);
        let token = token.clone();
        Arc::new(move |inputs| {
            clock.advance(10_000_000);
            let x = as_number(inputs[0].as_ref().unwrap());
            if x >= 3.0 {
                token.cancel(); // cancel mid-fan, deterministically
            }
            Ok(vec![number(x)])
        })
    };
    let elements: Vec<f64> = (0..50).map(f64::from).collect();
    let graph = SolveGraph::new(vec![decl(
        "mapped",
        "fake.coldcal",
        vec![Input::Value(number_list(&elements, None))],
        vec![1],
        run,
    )])
    .unwrap();
    let report = rig
        .scheduler
        .solve(&graph, &[NodeId(0)], 0, &token, &Recorder::default())
        .unwrap();
    assert!(report.cancelled);
    let stats = rig
        .scheduler
        .store()
        .stats("fake.coldcal")
        .expect("partial work IS calibration data");
    assert_eq!(stats.per_element_nanos(), Some(10_000_000));
}

#[test]
fn empty_fan_produces_empty_lists_not_errors() {
    let rig = rig(2, |_| {});
    let graph = SolveGraph::new(vec![decl(
        "mapped",
        "fake.empty",
        vec![Input::Value(number_list(&[], None))],
        vec![1],
        fake_fn(&rig.clock, 1_000, |x| x),
    )])
    .unwrap();
    let report = solve(&rig.scheduler, &graph, &[NodeId(0)], &Recorder::default());
    let NodeOutcome::Computed {
        outputs,
        elements: 0,
        ..
    } = report.outcome(NodeId(0))
    else {
        panic!("empty map computes an empty list")
    };
    let value = rig.scheduler.store().load_value(&outputs[0]).unwrap();
    let ValueData::List(list) = value.data() else {
        panic!("fan output is a list")
    };
    assert!(list.slots.is_empty());
}
