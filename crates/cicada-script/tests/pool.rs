//! Worker-pool contracts (doc 15 stage-4 DoD): describe + invoke round
//! the `MessagePack` boundary with pure-stdlib Python (no packages),
//! script failures carry tracebacks, and kill-the-worker cancellation
//! works. Regression coverage from the stage-4 adversarial review: full
//! msgpack integer family, large arrays/strings both directions,
//! Vector/Domain surviving the boundary, and toolchain-versioned source
//! hashing.
//!
//! Python 3 on PATH (or `CICADA_PYTHON`) is a dev/CI requirement for
//! these tests — a missing interpreter FAILS them loudly (the script host
//! is stage-4 scope, not optional).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use cicada_core::value::{HashedValue, List, ValueData};
use cicada_script::{KillSwitch, ScriptError, WorkerPool};

const FIXTURE: &str = r#"
import cicada

@cicada.node(title="Double All", description="each number, doubled.")
def double_all(values: "[Number]", gain: "Number" = 2.0) -> "[Number]":
    return [None if v is None else v * gain for v in values]

@cicada.node(title="Sum Coords", description="sum of point coordinates.")
def sum_coords(points: "[Point]") -> "[Number]":
    return [x + y + z for (x, y, z) in points]

@cicada.node(title="Echo Int", description="the integer back.")
def echo_int(n: "Integer") -> "Integer":
    return n

@cicada.node(title="Wide", description="n numbers.")
def wide(n: "Integer") -> "[Number]":
    return [float(i) for i in range(n)]

@cicada.node(title="Shout", description="text, repeated.")
def shout(text: "Text", times: "Integer") -> "Text":
    return text * times

@cicada.node(title="Flip", description="vector negated.")
def flip(v: "Vector", d: "Domain") -> "Vector":
    assert isinstance(v, cicada.Vector), type(v)
    assert isinstance(d, cicada.Domain), type(d)
    return cicada.Vector(-v.x, -v.y, -v.z)

@cicada.node(title="Explode", description="always raises.")
def explode(x: "Number") -> "Number":
    raise ValueError("boom from python")

@cicada.node(title="Spin", description="never returns.")
def spin(x: "Number") -> "Number":
    while True:
        pass
"#;

fn label() -> &'static Path {
    Path::new("pool_fixture.py")
}

fn number(x: f64) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Number(x)).unwrap()
}

fn integer(i: i64) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Integer(i)).unwrap()
}

fn number_list(values: &[Option<f64>]) -> Arc<HashedValue> {
    HashedValue::new(ValueData::List(List {
        axis: None,
        slots: values.iter().map(|v| v.map(number)).collect(),
    }))
    .unwrap()
}

fn invoke_one(
    pool: &WorkerPool,
    fn_name: &str,
    inputs: &BTreeMap<String, Arc<HashedValue>>,
) -> Result<Arc<HashedValue>, ScriptError> {
    pool.invoke(label(), FIXTURE, fn_name, inputs, &KillSwitch::new())
}

#[test]
fn describe_reports_signature_ports_defaults_and_toolchain() {
    let pool = WorkerPool::new().expect("pool");
    let described = pool.describe(label(), FIXTURE).expect("describe");
    assert!(
        described.python_version.starts_with('3'),
        "sys.version reported: {}",
        described.python_version
    );
    let names: Vec<&str> = described.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "double_all",
            "echo_int",
            "explode",
            "flip",
            "shout",
            "spin",
            "sum_coords",
            "wide"
        ]
    );
    let double = &described.nodes[0];
    assert_eq!(double.title, "Double All");
    assert_eq!(double.output, "[Number]");
    assert_eq!(double.inputs[0].ty, "[Number]");
    assert!(double.inputs[0].default.is_none(), "values is required");
    let gain = &double.inputs[1];
    assert_eq!(
        *gain.default.as_ref().unwrap().data(),
        ValueData::Number(2.0)
    );
}

#[test]
fn invoke_rounds_values_with_holes_preserved() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([(
        "values".to_owned(),
        number_list(&[Some(1.5), None, Some(-3.0)]),
    )]);
    let out = invoke_one(&pool, "double_all", &inputs).expect("invokes");
    let ValueData::List(list) = out.data() else {
        panic!("list out")
    };
    assert_eq!(list.slots.len(), 3);
    assert_eq!(
        *list.slots[0].as_ref().unwrap().data(),
        ValueData::Number(3.0)
    );
    assert!(list.slots[1].is_none(), "hole survives the round trip");
    assert_eq!(
        *list.slots[2].as_ref().unwrap().data(),
        ValueData::Number(-6.0)
    );
}

#[test]
fn omitted_optional_port_takes_the_python_default() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([("values".to_owned(), number_list(&[Some(2.0)]))]);
    let out = invoke_one(&pool, "double_all", &inputs).expect("invokes");
    let ValueData::List(list) = out.data() else {
        panic!("list out")
    };
    assert_eq!(
        *list.slots[0].as_ref().unwrap().data(),
        ValueData::Number(4.0)
    );
}

#[test]
fn point_marshalling_crosses_both_ways() {
    let pool = WorkerPool::new().expect("pool");
    let points = HashedValue::new(ValueData::List(List {
        axis: None,
        slots: vec![Some(
            HashedValue::new(ValueData::Point(cicada_core::spatial::Point::new(
                1.0, 2.0, 3.0,
            )))
            .unwrap(),
        )],
    }))
    .unwrap();
    let inputs = BTreeMap::from([("points".to_owned(), points)]);
    let out = invoke_one(&pool, "sum_coords", &inputs).expect("invokes");
    let ValueData::List(list) = out.data() else {
        panic!("list out")
    };
    assert_eq!(
        *list.slots[0].as_ref().unwrap().data(),
        ValueData::Number(6.0)
    );
}

// Regression (adversarial review, stage 4): rmpv emits the most COMPACT
// msgpack integer marker (uint8/16/32, int8/16/32); the worker's decoder
// only knew fixint + 64-bit forms, so any Integer outside [-32, 127]
// killed the worker with an unattributable I/O error.
#[test]
fn every_integer_marker_family_round_trips() {
    let pool = WorkerPool::new().expect("pool");
    for n in [
        0_i64,
        100,
        200,
        1_000,
        100_000,
        1 << 40,
        -33,
        -100,
        -100_000,
    ] {
        let inputs = BTreeMap::from([("n".to_owned(), integer(n))]);
        let out = invoke_one(&pool, "echo_int", &inputs)
            .unwrap_or_else(|error| panic!("echo_int({n}) failed: {error}"));
        assert_eq!(*out.data(), ValueData::Integer(n), "n = {n}");
    }
}

// Regression (adversarial review, stage 4): the worker's encoder had no
// array32/str32 escapes — a script returning >= 65536 elements (a field
// solve over a big grid) or a >= 64 KiB string crashed the worker.
#[test]
fn large_array_and_string_outputs_survive() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([("n".to_owned(), integer(70_000))]);
    let out = invoke_one(&pool, "wide", &inputs).expect("70k-element return");
    let ValueData::List(list) = out.data() else {
        panic!("list out")
    };
    assert_eq!(list.slots.len(), 70_000);
    assert_eq!(
        *list.slots[69_999].as_ref().unwrap().data(),
        ValueData::Number(69_999.0)
    );

    let inputs = BTreeMap::from([
        (
            "text".to_owned(),
            HashedValue::new(ValueData::Text(Arc::from("abcdefgh"))).unwrap(),
        ),
        ("times".to_owned(), integer(10_000)),
    ]);
    let out = invoke_one(&pool, "shout", &inputs).expect("80 KB text return");
    let ValueData::Text(text) = out.data() else {
        panic!("text out")
    };
    assert_eq!(text.len(), 80_000);
}

// Regression (adversarial review, stage 4): Vector and Domain used to
// cross into Python as bare tuples and come back retagged as Point (or
// unmarshallable) — the exact Point/Vector conflation the value model
// exists to kill.
#[test]
fn vector_and_domain_survive_the_boundary() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([
        (
            "v".to_owned(),
            HashedValue::new(ValueData::Vector(cicada_core::spatial::Vector::new(
                1.0, -2.0, 3.0,
            )))
            .unwrap(),
        ),
        (
            "d".to_owned(),
            HashedValue::new(ValueData::Domain(cicada_core::scalar::Domain::new(
                0.0, 5.0,
            )))
            .unwrap(),
        ),
    ]);
    let out = invoke_one(&pool, "flip", &inputs).expect("flips");
    assert_eq!(
        *out.data(),
        ValueData::Vector(cicada_core::spatial::Vector::new(-1.0, 2.0, -3.0)),
        "a Vector comes back a Vector, never a Point"
    );
}

#[test]
fn script_exception_is_a_loud_error_with_the_message() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([("x".to_owned(), number(1.0))]);
    let error = invoke_one(&pool, "explode", &inputs).expect_err("must fail");
    let ScriptError::Script(message) = &error else {
        panic!("script error, got {error:?}")
    };
    assert!(
        message.contains("boom from python"),
        "traceback carried: {message}"
    );
}

// Doc 15 stage-4 DoD: kill-the-worker cancellation works. The fixture
// spins forever; the switch is thrown from another thread as soon as the
// call is in flight; the call must come back Cancelled (bounded by the
// harness timeout — no sleeps in the test itself).
#[test]
fn kill_the_worker_cancels_a_spinning_script() {
    let pool = WorkerPool::new().expect("pool");
    let kill = KillSwitch::new();
    let killer = kill.clone();
    std::thread::scope(|scope| {
        scope.spawn(move || killer.kill());
        let inputs = BTreeMap::from([("x".to_owned(), number(1.0))]);
        let error = pool
            .invoke(label(), FIXTURE, "spin", &inputs, &kill)
            .expect_err("must cancel");
        assert!(
            matches!(error, ScriptError::Cancelled),
            "cancelled, got {error:?}"
        );
    });
    // The pool survives a kill: the next call gets a fresh worker.
    let inputs = BTreeMap::from([("values".to_owned(), number_list(&[Some(1.0)]))]);
    let out = invoke_one(&pool, "double_all", &inputs).expect("fresh worker serves the next call");
    assert!(matches!(out.data(), ValueData::List(_)));
}

#[test]
fn source_hash_changes_with_content_and_toolchain() {
    let a = cicada_script::source_hash(b"def f(): pass", "3.10");
    let b = cicada_script::source_hash(b"def f(): return 1", "3.10");
    let c = cicada_script::source_hash(b"def f(): pass", "3.12");
    assert_ne!(a, b, "source changes the hash");
    assert_ne!(a, c, "the interpreter version changes the hash (docs/12)");
    assert_eq!(a, cicada_script::source_hash(b"def f(): pass", "3.10"));
}
