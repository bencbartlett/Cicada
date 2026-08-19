//! Worker-pool contracts (doc 15 stage-4 DoD): describe + invoke round
//! the `MessagePack` boundary with pure-stdlib Python (no packages),
//! script failures carry tracebacks, and kill-the-worker cancellation
//! works. Regression coverage from the stage-4 adversarial review: full
//! msgpack integer family, large arrays/strings both directions,
//! Vector/Domain surviving the boundary, and toolchain-versioned source
//! hashing. Stage 6 (the wall slice's script ABI): msgpack bin both
//! ways, Mesh/Plane/Curve crossing both ways through the `cicada`
//! helpers, the `effectful` flag, multi-output dict returns and `-> None`
//! (with the refusals), and the 7,200-mesh crossing budget.
//!
//! Python 3 on PATH (or `CICADA_PYTHON`) is a dev/CI requirement for
//! these tests — a missing interpreter FAILS them loudly (the script host
//! is stage-4 scope, not optional).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_core::value::{HashedValue, List, ValueData};
use cicada_script::{KillSwitch, OutputDesc, ScriptError, WorkerPool};

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

# ---- stage 6: geometry, multi-output, effectful ----

@cicada.node(title="Shift Mesh", description="mesh translated by dx; vertices/triangles via the helpers.")
def shift_mesh(mesh: "Mesh", dx: "Number" = 1.0) -> "Mesh":
    assert isinstance(mesh, cicada.Mesh), type(mesh)
    moved = [(x + dx, y, z) for (x, y, z) in mesh.vertices]
    return cicada.Mesh.from_triangles(moved, mesh.triangles)

@cicada.node(title="Mesh Stats", description="vertex and triangle counts, plus the first vertex.")
def mesh_stats(mesh: "Mesh") -> {"vertices": "Integer", "triangles": "Integer", "first": "Point"}:
    return {
        "vertices": mesh.vertex_count,
        "triangles": mesh.triangle_count,
        "first": mesh.vertices[0],
    }

@cicada.node(title="Echo Geometry", description="planes and curves back, after type checks.")
def echo_geometry(plane: "Plane", curves: "[Curve]") -> {"plane": "Plane", "curves": "[Curve]"}:
    assert isinstance(plane, cicada.Plane), type(plane)
    assert isinstance(plane.x, cicada.Vector), type(plane.x)
    kinds = (cicada.Polyline, cicada.Line, cicada.Circle, cicada.Rectangle)
    for c in curves:
        assert isinstance(c, kinds), type(c)
    return {"plane": plane, "curves": curves}

@cicada.node(title="Many Meshes", description="n copies of a ~100-vertex mesh, as arrays.")
def many_meshes(n: "Integer", vertices: "Integer" = 100) -> "[Mesh]":
    from array import array
    positions = array("d")
    for i in range(vertices):
        positions.extend((float(i), float(i % 7), float(i % 3)))
    indices = array("I")
    for i in range(0, vertices - 2):
        indices.extend((i, i + 1, i + 2))
    return [cicada.Mesh(positions, indices) for _ in range(n)]

@cicada.node(title="Mesh Vertex Sum", description="sum of every coordinate over a mesh list, via arrays.")
def mesh_vertex_sum(meshes: "[Mesh]") -> {"total": "Number", "count": "Integer"}:
    total = 0.0
    for m in meshes:
        total += sum(m.positions)
    return {"total": total, "count": len(meshes)}

@cicada.node(title="Missing Key", description="forgets an output.")
def missing_key(x: "Number") -> {"a": "Number", "b": "Number"}:
    return {"a": x}

@cicada.node(title="Extra Key", description="invents an output.")
def extra_key(x: "Number") -> {"a": "Number"}:
    return {"a": x, "zzz": 1.0}

@cicada.node(title="Not A Dict", description="returns a bare value for a multi-output node.")
def not_a_dict(x: "Number") -> {"a": "Number", "b": "Number"}:
    return x

@cicada.node(title="Note", description="writes a file; no outputs.", effectful=True)
def note(path: "Text", text: "Text") -> None:
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)

@cicada.node(title="Chatty None", description="declared -> None but returns a value.", effectful=True)
def chatty_none(x: "Number") -> None:
    return x

@cicada.node(title="Bin Echo", description="arrays back as-is (bin both ways).")
def bin_echo(mesh: "Mesh") -> "Mesh":
    from array import array
    assert isinstance(mesh.positions, array) and mesh.positions.typecode == "d"
    assert isinstance(mesh.indices, array) and mesh.indices.itemsize == 4
    return mesh
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
    let mut outputs = pool.invoke(label(), FIXTURE, fn_name, inputs, &KillSwitch::new())?;
    assert_eq!(outputs.len(), 1, "single-output node returns one value");
    Ok(outputs.remove(0))
}

fn invoke_many(
    pool: &WorkerPool,
    fn_name: &str,
    inputs: &BTreeMap<String, Arc<HashedValue>>,
) -> Result<Vec<Arc<HashedValue>>, ScriptError> {
    pool.invoke(label(), FIXTURE, fn_name, inputs, &KillSwitch::new())
}

fn text(s: &str) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Text(Arc::from(s))).unwrap()
}

fn mesh(m: Mesh) -> Arc<HashedValue> {
    HashedValue::new(ValueData::Mesh(m)).unwrap()
}

fn tetrahedron() -> Mesh {
    Mesh::new(
        vec![
            0.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, //
            0.0, 0.0, 1.0,
        ],
        vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
    )
    .unwrap()
}

fn tilted_plane() -> Plane {
    Plane {
        origin: Point::new(1.0, 2.0, 3.0),
        x: Vector::new(0.0, 1.0, 0.0),
        y: Vector::new(-1.0, 0.0, 0.0),
    }
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
    assert!(
        names.windows(2).all(|pair| pair[0] < pair[1]),
        "nodes arrive sorted by name: {names:?}"
    );
    assert!(names.contains(&"double_all") && names.contains(&"wide"));
    let double = &described.nodes[names.iter().position(|n| *n == "double_all").unwrap()];
    assert_eq!(double.title, "Double All");
    assert!(!double.effectful, "pure by default");
    assert_eq!(double.outputs.len(), 1);
    assert_eq!(
        double.outputs[0],
        OutputDesc {
            name: "out".to_owned(),
            ty: "[Number]".to_owned()
        }
    );
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

// ---- stage 6 ----

#[test]
fn describe_reports_effectful_flag_and_output_shapes() {
    let pool = WorkerPool::new().expect("pool");
    let described = pool.describe(label(), FIXTURE).expect("describe");
    let by_name = |name: &str| {
        described
            .nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} described"))
    };
    let stats = by_name("mesh_stats");
    assert!(!stats.effectful);
    let outputs: Vec<(&str, &str)> = stats
        .outputs
        .iter()
        .map(|o| (o.name.as_str(), o.ty.as_str()))
        .collect();
    assert_eq!(
        outputs,
        [
            ("vertices", "Integer"),
            ("triangles", "Integer"),
            ("first", "Point")
        ],
        "dict insertion order = port order"
    );
    let note = by_name("note");
    assert!(note.effectful, "effectful=True is reported");
    assert!(note.outputs.is_empty(), "-> None declares no outputs");
    assert_eq!(note.inputs.len(), 2);
}

#[test]
fn mesh_crosses_both_ways_through_the_helpers() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([
        ("mesh".to_owned(), mesh(tetrahedron())),
        ("dx".to_owned(), number(2.5)),
    ]);
    let out = invoke_one(&pool, "shift_mesh", &inputs).expect("invokes");
    let ValueData::Mesh(shifted) = out.data() else {
        panic!("mesh out")
    };
    assert_eq!(
        shifted.indices(),
        tetrahedron().indices(),
        "topology preserved"
    );
    assert_eq!(&shifted.positions()[0..6], &[2.5, 0.0, 0.0, 3.5, 0.0, 0.0]);
    assert!(shifted.is_watertight());

    // bin in → bin out, byte-identical value (hash-identical).
    let inputs = BTreeMap::from([("mesh".to_owned(), mesh(tetrahedron()))]);
    let out = invoke_one(&pool, "bin_echo", &inputs).expect("invokes");
    assert_eq!(
        out.hash(),
        mesh(tetrahedron()).hash(),
        "round trip is hash-identical"
    );
}

#[test]
fn plane_and_every_curve_variant_round_trip_hash_identical() {
    let pool = WorkerPool::new().expect("pool");
    let plane = tilted_plane();
    let curves = vec![
        Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(0.5, 2.0, -1.0),
            ],
            closed: true,
        }),
        Curve::Polyline(Polyline {
            vertices: vec![Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0)],
            closed: false,
        }),
        Curve::Line(Line {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 1.0, 1.0),
        }),
        Curve::Circle(Circle { plane, radius: 2.5 }),
        Curve::Rectangle(Rectangle {
            plane,
            x: Domain::new(-1.0, 2.0),
            y: Domain::new(0.0, 0.5),
        }),
    ];
    let curve_list = HashedValue::new(ValueData::List(List {
        axis: None,
        slots: curves
            .into_iter()
            .map(|c| Some(HashedValue::new(ValueData::Curve(c)).unwrap()))
            .collect(),
    }))
    .unwrap();
    let plane_value = HashedValue::new(ValueData::Plane(plane)).unwrap();
    let inputs = BTreeMap::from([
        ("plane".to_owned(), Arc::clone(&plane_value)),
        ("curves".to_owned(), Arc::clone(&curve_list)),
    ]);
    let outs = invoke_many(&pool, "echo_geometry", &inputs).expect("invokes");
    assert_eq!(outs.len(), 2);
    assert_eq!(outs[0].hash(), plane_value.hash(), "Plane round trip");
    assert_eq!(
        outs[1].hash(),
        curve_list.hash(),
        "every curve variant round trips"
    );
}

#[test]
fn multi_output_dict_return_maps_to_declared_order() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([("mesh".to_owned(), mesh(tetrahedron()))]);
    let outs = invoke_many(&pool, "mesh_stats", &inputs).expect("invokes");
    assert_eq!(outs.len(), 3);
    assert_eq!(*outs[0].data(), ValueData::Integer(4));
    assert_eq!(*outs[1].data(), ValueData::Integer(4));
    assert_eq!(*outs[2].data(), ValueData::Point(Point::new(0.0, 0.0, 0.0)));
}

#[test]
fn multi_output_shape_lies_are_refused_with_counts() {
    let pool = WorkerPool::new().expect("pool");
    let inputs = BTreeMap::from([("x".to_owned(), number(1.0))]);
    let error = invoke_many(&pool, "missing_key", &inputs).expect_err("missing key refuses");
    let ScriptError::Script(message) = &error else {
        panic!("script error, got {error:?}")
    };
    assert!(
        message.contains("1 missing [b]") && message.contains("0 extra []"),
        "{message}"
    );
    let error = invoke_many(&pool, "extra_key", &inputs).expect_err("extra key refuses");
    let ScriptError::Script(message) = &error else {
        panic!("script error, got {error:?}")
    };
    assert!(
        message.contains("0 missing []") && message.contains("1 extra [zzz]"),
        "{message}"
    );
    let error = invoke_many(&pool, "not_a_dict", &inputs).expect_err("non-dict refuses");
    let ScriptError::Script(message) = &error else {
        panic!("script error, got {error:?}")
    };
    assert!(
        message.contains("must return a dict with exactly those keys") && message.contains("float"),
        "{message}"
    );
}

#[test]
fn none_return_runs_the_effect_and_yields_no_outputs() {
    let pool = WorkerPool::new().expect("pool");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    let inputs = BTreeMap::from([
        ("path".to_owned(), text(&path.to_string_lossy())),
        ("text".to_owned(), text("hello from python")),
    ]);
    let outs = invoke_many(&pool, "note", &inputs).expect("invokes");
    assert!(outs.is_empty(), "-> None yields no output values");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello from python");

    let inputs = BTreeMap::from([("x".to_owned(), number(1.0))]);
    let error =
        invoke_many(&pool, "chatty_none", &inputs).expect_err("a value for -> None refuses");
    let ScriptError::Script(message) = &error else {
        panic!("script error, got {error:?}")
    };
    assert!(message.contains("declared `-> None`"), "{message}");
}

// Budget (stage-6 contract §1): a 7,200-mesh list of ~100-vertex meshes
// must cross Python → Rust in well under a second. The test asserts the
// data (counts, a sampled vertex, hash-stability of the list across two
// crossings) and PRINTS the timing (`--nocapture`) — a wall-clock
// assertion would be a flaky test, not a budget; the number is reported
// in the stage notes instead.
#[test]
fn seven_thousand_meshes_cross_python_to_rust() {
    let pool = WorkerPool::new().expect("pool");
    // Warm the worker (process start + module compile are not the crossing).
    let warm = BTreeMap::from([("n".to_owned(), integer(1))]);
    invoke_one(&pool, "many_meshes", &warm).expect("warm");

    let inputs = BTreeMap::from([("n".to_owned(), integer(7_200))]);
    let start = std::time::Instant::now();
    let out = invoke_one(&pool, "many_meshes", &inputs).expect("7,200 meshes cross");
    let elapsed = start.elapsed();
    let ValueData::List(list) = out.data() else {
        panic!("list out")
    };
    assert_eq!(list.slots.len(), 7_200);
    let ValueData::Mesh(first) = list.slots[0].as_ref().unwrap().data() else {
        panic!("mesh element")
    };
    assert_eq!(first.vertex_count(), 100);
    assert_eq!(first.triangle_count(), 98);
    assert_eq!(first.positions()[3..6], [1.0, 1.0, 1.0]);
    let ValueData::Mesh(final_mesh) = list.slots[7_199].as_ref().unwrap().data() else {
        panic!("mesh element")
    };
    assert_eq!(final_mesh, first, "every copy is the same mesh");
    println!(
        "7,200 meshes x 100 vertices Python->Rust (invoke + unmarshal + hash): {:.1} ms",
        elapsed.as_secs_f64() * 1e3
    );

    // And the other direction: the same list back into Python, summed
    // through array arithmetic (no per-float Python on the hot path).
    let inputs = BTreeMap::from([("meshes".to_owned(), Arc::clone(&out))]);
    let start = std::time::Instant::now();
    let outs = invoke_many(&pool, "mesh_vertex_sum", &inputs).expect("7,200 meshes cross back");
    let elapsed_back = start.elapsed();
    assert_eq!(*outs[1].data(), ValueData::Integer(7_200));
    let per_mesh: f64 = (0..100)
        .map(|i: i32| f64::from(i) + f64::from(i % 7) + f64::from(i % 3))
        .sum();
    assert_eq!(*outs[0].data(), ValueData::Number(per_mesh * 7_200.0));
    println!(
        "7,200 meshes x 100 vertices Rust->Python (marshal + invoke): {:.1} ms",
        elapsed_back.as_secs_f64() * 1e3
    );
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
