//! The sharing model under the SCHEDULER (v0.1 item 3 WP-B review): the
//! seam's own rayon test proves op-local handles are safe on a pool; this
//! one proves it where it matters — a `SolveGraph` whose nodes call
//! `cicada_geom::solid` through the executor's wavefront and `each()`
//! fan-out, on a pool with threads > 1, against the same graph solved on
//! one thread. The stdlib's OCCT-backed nodes are WP-C's; until then the
//! graph's nodes are closures over the value-level API, exactly what those
//! nodes will be.
//!
//! Every hash matches the single-threaded run and the direct computation.
//! The kernel is the product (cicada-geom's `occt` feature is on by
//! default and cicada-server links it unconditionally), so there is no
//! kernel-free arm here: the test asserts the kernel is present rather than
//! passing vacuously in a build that lacks it. The kernel-free world is
//! `cargo test -p cicada-geom --no-default-features`.
//!
//! This lives in `cicada-server` because it is the lowest crate that
//! depends on both `cicada-geom` and `cicada-sched` (the dependency DAG
//! forbids an edge between them).

// Tests are exempt from the unwrap/expect denial, but the exemption only
// recognizes #[test] fns — not helpers in integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_core::spatial::{Point, Vector};
use cicada_core::value::{HashedValue, List, ValueData};
use cicada_geom::solid::{self, Deflection};
use cicada_sched::{
    CancelToken, DiskStore, Event, Input, NodeDecl, NodeError, NodeFn, NodeId, NodeOutcome,
    Observer, Scheduler, SchedulerConfig, SolveGraph, VirtualClock,
};

const TOL: f64 = 1e-6;
/// Cutters per run — enough that the executor's spread cap
/// (`n.div_ceil(workers)`) splits the fan-out into one chunk per worker.
const CUTTERS: usize = 48;

fn number(x: f64) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Number(x)).unwrap()
}

fn solid_value(solid: Solid) -> Result<Arc<HashedValue>, NodeError> {
    HashedValue::new(ValueData::Solid(solid)).map_err(|e| NodeError::new(e.to_string()))
}

fn expect_solid(value: &HashedValue) -> Result<&Solid, NodeError> {
    match value.data() {
        ValueData::Solid(solid) => Ok(solid),
        other => Err(NodeError::new(format!(
            "expected a Solid, got {}",
            other.kind_name()
        ))),
    }
}

fn expect_number(value: &HashedValue) -> Result<f64, NodeError> {
    match value.data() {
        ValueData::Number(x) => Ok(*x),
        other => Err(NodeError::new(format!(
            "expected a Number, got {}",
            other.kind_name()
        ))),
    }
}

fn required(inputs: &[Option<Arc<HashedValue>>], port: usize) -> Result<&HashedValue, NodeError> {
    inputs
        .get(port)
        .and_then(Option::as_deref)
        .ok_or_else(|| NodeError::new(format!("port {port} has no value")))
}

/// The cutter through the block at offset `x`: a 0.1-wide slot along Z.
fn cutter_at(x: f64) -> Result<Solid, cicada_geom::GeomError> {
    solid::extrude_polygon(
        &[
            Point::new(x, 2.0, -5.0),
            Point::new(x + 0.1, 2.0, -5.0),
            Point::new(x + 0.1, 17.0, -5.0),
            Point::new(x, 17.0, -5.0),
        ],
        Vector::new(0.0, 0.0, 40.0),
        TOL,
    )
}

/// Disjoint slot offsets across the block's X extent [0, 10]: pitch 0.2,
/// width 0.1 (`cutter_at`), the last ending at 9.65.
fn offsets() -> Vec<f64> {
    #[allow(clippy::cast_precision_loss)]
    (0..CUTTERS).map(|i| 0.15 + 0.2 * i as f64).collect()
}

/// The graph: `block` (a bare kernel call) → `holes` = each(cutters) cut
/// from it; `holes_b` the same cut under a different op name so two nodes
/// cut the SAME block bytes at once; `block_mesh` tessellates the block
/// while the cuts run; `hole_meshes` tessellates every hole.
fn graph() -> SolveGraph {
    let deflection = Deflection::display(&ProjectConfig::default());
    let block: NodeFn = Arc::new(|_ctx, _inputs| {
        let block = solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0))
            .map_err(|e| NodeError::new(e.to_string()))?;
        Ok(vec![solid_value(block)?])
    });
    let cutter: NodeFn = Arc::new(|_ctx, inputs| {
        let x = expect_number(required(inputs, 0)?)?;
        let cutter = cutter_at(x).map_err(|e| NodeError::new(e.to_string()))?;
        Ok(vec![solid_value(cutter)?])
    });
    let difference: NodeFn = Arc::new(|_ctx, inputs| {
        let block = expect_solid(required(inputs, 0)?)?;
        let cutter = expect_solid(required(inputs, 1)?)?;
        let hole = solid::difference(block, cutter).map_err(|e| NodeError::new(e.to_string()))?;
        Ok(vec![solid_value(hole)?])
    });
    let tessellate: NodeFn = Arc::new(move |_ctx, inputs| {
        let solid = expect_solid(required(inputs, 0)?)?;
        let tessellation =
            solid::tessellate(solid, deflection).map_err(|e| NodeError::new(e.to_string()))?;
        Ok(vec![
            HashedValue::new(ValueData::Mesh(tessellation.mesh.0))
                .map_err(|e| NodeError::new(e.to_string()))?,
        ])
    });
    let offsets = HashedValue::new(ValueData::List(List {
        axis: None,
        slots: offsets().into_iter().map(|x| Some(number(x))).collect(),
    }))
    .unwrap();
    let decl = |name: &str, op: &str, inputs: Vec<Input>, fan: Vec<u8>, run: &NodeFn| NodeDecl {
        name: name.to_owned(),
        op: op.to_owned(),
        version: 1,
        body_hash: None,
        tolerance: None,
        inputs,
        fan,
        output_count: 1,
        effectful: false,
        volatile: false,
        run: Arc::clone(run),
    };
    let port = |node: usize| Input::Port {
        node: NodeId(node),
        output: 0,
    };
    SolveGraph::new(vec![
        decl("block", "test.box", vec![], vec![], &block), // 0
        decl(
            "cutters",
            "test.cutter",
            vec![Input::Value(offsets)],
            vec![1],
            &cutter,
        ), // 1
        decl(
            "holes",
            "test.difference",
            vec![port(0), port(1)],
            vec![0, 1],
            &difference,
        ), // 2
        decl(
            "holes_b",
            "test.difference_b",
            vec![port(0), port(1)],
            vec![0, 1],
            &difference,
        ), // 3
        decl(
            "block_mesh",
            "test.tessellate",
            vec![port(0)],
            vec![0],
            &tessellate,
        ), // 4
        decl(
            "hole_meshes",
            "test.tessellate_each",
            vec![port(2)],
            vec![1],
            &tessellate,
        ), // 5
    ])
    .unwrap()
}

/// Counts the chunks the `holes` fan-out was split into — the evidence
/// that its elements ran on several workers rather than one.
#[derive(Default)]
struct ChunkCounter {
    holes_chunks: AtomicUsize,
}

impl Observer for ChunkCounter {
    fn on_event(&self, event: &Event<'_>) {
        if let Event::ChunkExecuted { node: "holes", .. } = event {
            self.holes_chunks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct Run {
    _dir: tempfile::TempDir,
    scheduler: Scheduler,
    report: cicada_sched::SolveReport,
    holes_chunks: usize,
}

fn solve_with(threads: usize) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let scheduler = Scheduler::new(
        Arc::new(store),
        // A virtual clock: the assertions are on bytes, never on time, and
        // the test rules forbid the wall clock (docs/14).
        Arc::new(VirtualClock::new()),
        SchedulerConfig {
            threads,
            ..SchedulerConfig::default()
        },
    )
    .unwrap();
    let graph = graph();
    let targets: Vec<NodeId> = (0..graph.len()).map(NodeId).collect();
    let counter = ChunkCounter::default();
    let report = scheduler
        .solve(&graph, &targets, 0, &CancelToken::new(), &counter)
        .unwrap();
    Run {
        _dir: dir,
        scheduler,
        report,
        holes_chunks: counter.holes_chunks.load(Ordering::Relaxed),
    }
}

#[test]
fn related_solids_are_safe_under_the_scheduler_with_threads_above_one() {
    // cicada-server links the kernel (cicada-geom's `occt` is a default
    // feature since WP-C); a build without it would make this test
    // vacuous, so it fails loudly instead.
    assert!(
        solid::kernel_available(),
        "this test needs the OCCT kernel (cicada-geom feature `occt`)"
    );

    let serial = solve_with(1);
    let parallel = solve_with(8);
    assert_eq!(parallel.scheduler.threads(), 8);
    assert_eq!(serial.scheduler.threads(), 1);
    // The fan-out really was spread: the executor's spread cap is
    // `n.div_ceil(workers)` = 6 elements per chunk for 48 on 8 workers, so
    // at least 8 chunks of cuts ran — on a pool of 8, concurrently.
    assert!(
        parallel.holes_chunks >= 8,
        "holes ran in {} chunk(s); the fan-out was not spread over the pool",
        parallel.holes_chunks
    );

    // Every node computed (a fresh store each), and every output hash is
    // the same on eight threads as on one.
    for id in 0..6 {
        let a = serial.report.outcome(NodeId(id));
        let b = parallel.report.outcome(NodeId(id));
        assert!(
            matches!(a, NodeOutcome::Computed { .. }),
            "serial node {id}: {a:?}"
        );
        assert!(
            matches!(b, NodeOutcome::Computed { .. }),
            "parallel node {id}: {b:?}"
        );
        assert_eq!(
            a.output_hashes().unwrap(),
            b.output_hashes().unwrap(),
            "node {id}: the bytes followed the thread count"
        );
    }

    // And the parallel run's values are what the direct, single-call
    // computation yields — not merely self-consistent.
    let block = solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).unwrap();
    let load = |id: usize| {
        let hash = parallel.report.outcome(NodeId(id)).output_hashes().unwrap()[0];
        parallel.scheduler.store().load_value(&hash).unwrap()
    };
    let ValueData::Solid(stored_block) = load(0).data().clone() else {
        panic!("block is a Solid")
    };
    assert_eq!(&stored_block, &block);

    let ValueData::List(holes) = load(2).data().clone() else {
        panic!("holes is a list")
    };
    let ValueData::List(holes_b) = load(3).data().clone() else {
        panic!("holes_b is a list")
    };
    assert_eq!(holes.slots.len(), CUTTERS);
    assert_eq!(holes_b.slots.len(), CUTTERS);
    for (i, x) in offsets().into_iter().enumerate() {
        let expected = solid::difference(&block, &cutter_at(x).unwrap()).unwrap();
        let ValueData::Solid(got) = holes.slots[i].as_ref().unwrap().data() else {
            panic!("hole {i} is a Solid")
        };
        assert_eq!(got, &expected, "hole {i}: drifted from the direct cut");
        assert_eq!(
            holes.slots[i].as_ref().unwrap().hash(),
            holes_b.slots[i].as_ref().unwrap().hash(),
            "hole {i}: the two concurrent cuts of the same block disagree"
        );
    }
    let ValueData::List(meshes) = load(5).data().clone() else {
        panic!("hole_meshes is a list")
    };
    assert_eq!(meshes.slots.len(), CUTTERS);
    let deflection = Deflection::display(&ProjectConfig::default());
    let expected_block_mesh = solid::tessellate(&block, deflection).unwrap().mesh.0;
    let ValueData::Mesh(block_mesh) = load(4).data().clone() else {
        panic!("block_mesh is a Mesh")
    };
    assert_eq!(block_mesh, expected_block_mesh);
    assert_eq!(block_mesh.triangle_count(), 12);
    // The block's bytes never changed under all of it.
    assert_eq!(
        block,
        solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).unwrap()
    );
}
