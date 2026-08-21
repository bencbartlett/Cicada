//! Cache hygiene across the tier flip (v0.1 item 3 WP-C, adversarial
//! review finding 1 — a blocker): the spike's `box` returned a
//! `Watertight<Mesh>`; WP-C's `box` returns a `Solid` under the SAME name,
//! the SAME port list and — before this test existed — the same
//! `version = 1`. The memo key is `blake3(epoch, op, version, tolerance,
//! input hashes, fan)` and the hit path checks arity only, so a store
//! warmed by a pre-flip engine served the mesh box to the Solid-typed
//! node: a GREEN node of the wrong kind, red one node downstream
//! (`volume`: "expected Solid, got Mesh") or not at all on a display sink.
//! Reproduced with main's engine and the branch's on one `--cache-dir`
//! over the 02-solids default (`span = 0..2`).
//!
//! The fix is the add-stdlib-node rule — a behavior change bumps
//! `version` — applied to the four flipped names. This test is the
//! regression: it plants, in a fresh store, the memo entry a pre-flip
//! engine would have recorded for `block = box(x=span, y=span, z=span)`
//! (the real lowered decl's op / tolerance / inputs / fan, `version = 1`,
//! the mesh box's value as the output), solves the real pipeline through
//! the real registry, and requires `block` to COMPUTE a `Solid`. A
//! reverted version (or a future flip that forgets the bump) makes
//! `block` a cache hit on the planted mesh and fails here.
//!
//! It lives in `cicada-server` because it is the lowest crate that holds
//! the registry, the `.cic` lowering and the scheduler together
//! (`cicada run` is a printer over the same path).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cicada_core::config::ProjectConfig;
use cicada_core::scalar::Domain;
use cicada_core::spatial::Plane;
use cicada_core::value::{HashedValue, ValueData};
use cicada_sched::{
    CancelToken, DiskStore, Input, KeyInputs, NodeOutcome, NoopObserver, Scheduler,
    SchedulerConfig, VirtualClock, node_key,
};
use cicada_server::compile;
use cicada_server::lower::{LoweredBinding, lower};
use cicada_server::scripts::ScriptCancel;
use cicada_stdlib::meshes::mesh_box::{MeshBoxIn, mesh_box};

const PIPELINE: &str = "# cicada 1
span = construct_domain(start=0.0, end=2.0)
block = box(x=span, y=span, z=span)
";

/// The version every pre-flip engine keyed the spike's `box` under.
const PRE_FLIP_VERSION: u32 = 1;

/// The real pipeline, lowered through the real registry.
struct Pipeline {
    lowered: cicada_server::lower::Lowered,
    block: cicada_sched::NodeId,
    config: ProjectConfig,
}

fn lower_pipeline(dir: &std::path::Path) -> Pipeline {
    let pipeline = dir.join("stale.cic");
    std::fs::write(&pipeline, PIPELINE).unwrap();
    let loaded = compile::load(&pipeline, PIPELINE, &ScriptCancel::new()).expect("loads");
    assert!(
        loaded.resolution.diagnostics.is_empty(),
        "{:?}",
        loaded.resolution.diagnostics
    );
    let targets = compile::resolve_targets(&loaded.document, &loaded.specs, &[]).unwrap();
    let config = ProjectConfig::default();
    let lowered = lower(
        &loaded.document,
        &loaded.resolution,
        &loaded.specs,
        &config,
        &targets.names,
        &loaded.scripts,
    )
    .expect("lowers");
    let Some(LoweredBinding::Port { node: block, .. }) = lowered.bindings.get("block") else {
        panic!("`block` lowers to a node output");
    };
    let block = *block;
    assert_eq!(lowered.graph.node(block).op, "box");
    Pipeline {
        lowered,
        block,
        config,
    }
}

/// The memo key an engine keys `block` under at `version`: the lowered
/// decl's own op, tolerance, inputs and fan — `span` is content-addressed,
/// so its hash is the Domain's.
fn key_at(lowered: &Pipeline, version: u32) -> cicada_sched::NodeKey {
    let decl = lowered.lowered.graph.node(lowered.block);
    let span = HashedValue::new(ValueData::Domain(Domain::new(0.0, 2.0))).unwrap();
    let input_hashes: Vec<_> = decl
        .inputs
        .iter()
        .map(|input| match input {
            Input::Value(value) => Some(value.hash()),
            Input::Absent => None,
            Input::Port { .. } => Some(span.hash()),
        })
        .collect();
    assert_eq!(input_hashes.len(), 4, "plane, x, y, z");
    assert!(input_hashes[0].is_none(), "the plane is the port default");
    node_key(&KeyInputs {
        op: &decl.op,
        version,
        body_hash: decl.body_hash.as_ref(),
        tolerance: decl.tolerance.as_ref(),
        inputs: &input_hashes,
        fan: &decl.fan,
    })
}

/// The value the spike's `box` produced for these inputs.
fn mesh_box_value(config: &ProjectConfig) -> Arc<HashedValue> {
    let mesh = mesh_box(
        config,
        MeshBoxIn {
            plane: Plane::world_xy(),
            x: Domain::new(0.0, 2.0),
            y: Domain::new(0.0, 2.0),
            z: Domain::new(0.0, 2.0),
        },
    );
    HashedValue::new(ValueData::Mesh(mesh.0)).unwrap()
}

/// Plant `planted` as `block`'s memo entry under `key` in a fresh store,
/// solve `block`, return the scheduler (for the store) and the outcome.
fn solve_with_planted(
    dir: &std::path::Path,
    lowered: &Pipeline,
    key: cicada_sched::NodeKey,
    planted: &Arc<HashedValue>,
) -> (Scheduler, NodeOutcome) {
    let (store, _) = DiskStore::open(dir).unwrap();
    store.store_value(planted).unwrap();
    store.record_memo(key, &[planted.hash()]).unwrap();
    assert!(store.memo(&key).is_some(), "the entry is planted");
    let scheduler = Scheduler::new(
        Arc::new(store),
        // A virtual clock: nothing here is timed (docs/14 forbids the wall
        // clock in tests).
        Arc::new(VirtualClock::new()),
        SchedulerConfig {
            threads: 1,
            ..SchedulerConfig::default()
        },
    )
    .unwrap();
    let report = scheduler
        .solve(
            &lowered.lowered.graph,
            &[lowered.block],
            0,
            &CancelToken::new(),
            &NoopObserver,
        )
        .unwrap();
    assert!(report.failures().is_empty(), "{:?}", report.failures());
    let outcome = report.outcome(lowered.block).clone();
    (scheduler, outcome)
}

#[test]
fn a_pre_flip_memo_entry_never_serves_the_solid_typed_box() {
    let dir = tempfile::tempdir().unwrap();
    let lowered = lower_pipeline(dir.path());
    let version = lowered.lowered.graph.node(lowered.block).version;
    assert!(
        version > PRE_FLIP_VERSION,
        "`box` is at version {version} — the tier flip changed its output kind under an \
         unchanged port list, so it must be keyed apart from the spike's mesh box \
         (add-stdlib-node: bump `version` on ANY behavior change)"
    );

    let mesh = mesh_box_value(&lowered.config);
    let (scheduler, outcome) = solve_with_planted(
        &dir.path().join("stale"),
        &lowered,
        key_at(&lowered, PRE_FLIP_VERSION),
        &mesh,
    );
    assert!(
        matches!(outcome, NodeOutcome::Computed { .. }),
        "`block` must compute past a pre-flip memo entry, not hit it: {outcome:?}"
    );
    let hash = outcome.output_hashes().unwrap()[0];
    assert_ne!(hash, mesh.hash(), "the planted mesh was served");
    let value = scheduler.store().load_value(&hash).unwrap();
    assert!(
        matches!(value.data(), ValueData::Solid(_)),
        "`box` yields a Solid, got {}",
        value.data().kind_name()
    );
}

// The control that keeps the test above from passing vacuously: the SAME
// planted entry under the CURRENT version IS served — so the key this test
// builds is the key the executor builds, and a missing bump really would
// have handed the mesh to the Solid-typed node.
#[test]
fn the_same_entry_at_the_current_version_is_a_cache_hit() {
    let dir = tempfile::tempdir().unwrap();
    let lowered = lower_pipeline(dir.path());
    let version = lowered.lowered.graph.node(lowered.block).version;
    let mesh = mesh_box_value(&lowered.config);
    let (_scheduler, outcome) = solve_with_planted(
        &dir.path().join("current"),
        &lowered,
        key_at(&lowered, version),
        &mesh,
    );
    assert!(
        matches!(outcome, NodeOutcome::CacheHit { .. }),
        "the planted entry at the node's own version must be the hit: {outcome:?}"
    );
    assert_eq!(
        outcome.output_hashes().unwrap()[0],
        mesh.hash(),
        "the memo promised the planted value"
    );
}
