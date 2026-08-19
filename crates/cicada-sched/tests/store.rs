//! Disk-store contracts (docs/12 §The store): content-addressed roundtrips
//! for every value kind (property-tested over nested Merkle trees),
//! hash-verified loads that refuse corruption loudly, and the
//! never-inside-the-project rule for the default cache location.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::sync::Arc;

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle};
use cicada_core::scalar::{Color, Domain, IndexMap};
use cicada_core::spatial::{Plane, Point, Vector, Xform};
use cicada_core::value::{HashedValue, List, ValueData};
use cicada_sched::{BlobLocation, DiskStore, PACK_MAX_BYTES, StoreError, project_cache_dir};
use proptest::prelude::*;

fn value(data: ValueData) -> Arc<HashedValue> {
    HashedValue::new(data).unwrap()
}

/// Finite f64s (NaN is refused at construction by design).
fn finite_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<f64>().prop_filter("finite", |x| x.is_finite()),
        Just(0.0),
        Just(-0.0),
        Just(f64::INFINITY),
    ]
}

fn leaf_value() -> impl Strategy<Value = Arc<HashedValue>> {
    prop_oneof![
        finite_f64().prop_map(|x| value(ValueData::Number(x))),
        any::<i64>().prop_map(|i| value(ValueData::Integer(i))),
        any::<bool>().prop_map(|b| value(ValueData::Boolean(b))),
        "[a-z0-9 ]{0,12}".prop_map(|s| value(ValueData::Text(Arc::from(s.as_str())))),
        (finite_f64(), finite_f64(), finite_f64(), finite_f64())
            .prop_map(|(r, g, b, a)| value(ValueData::Color(Color::new(r, g, b, a)))),
        (finite_f64(), finite_f64()).prop_map(|(s, e)| value(ValueData::Domain(Domain::new(s, e)))),
        proptest::collection::vec(any::<u64>(), 0..6)
            .prop_map(|v| value(ValueData::IndexMap(IndexMap(v)))),
        (finite_f64(), finite_f64(), finite_f64())
            .prop_map(|(x, y, z)| value(ValueData::Point(Point::new(x, y, z)))),
        (finite_f64(), finite_f64(), finite_f64())
            .prop_map(|(x, y, z)| value(ValueData::Vector(Vector::new(x, y, z)))),
        Just(value(ValueData::Plane(Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        }))),
        Just(value(ValueData::Xform(Xform::identity()))),
        Just(value(ValueData::Nothing)),
        curve_value(),
        mesh_value(),
    ]
}

fn finite_point() -> impl Strategy<Value = Point> {
    (finite_f64(), finite_f64(), finite_f64()).prop_map(|(x, y, z)| Point::new(x, y, z))
}

/// Arbitrary curves across all four analytic variants (stage 4).
fn curve_value() -> impl Strategy<Value = Arc<HashedValue>> {
    let plane = || {
        finite_point().prop_map(|origin| Plane {
            origin,
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        })
    };
    prop_oneof![
        (finite_point(), finite_point())
            .prop_map(|(a, b)| value(ValueData::Curve(Curve::Line(Line { a, b })))),
        (
            proptest::collection::vec(finite_point(), 0..8),
            any::<bool>()
        )
            .prop_map(|(vertices, closed)| value(ValueData::Curve(Curve::Polyline(
                Polyline { vertices, closed }
            )))),
        (plane(), finite_f64()).prop_map(|(plane, radius)| value(ValueData::Curve(Curve::Circle(
            Circle { plane, radius }
        )))),
        (
            plane(),
            finite_f64(),
            finite_f64(),
            finite_f64(),
            finite_f64()
        )
            .prop_map(
                |(plane, x0, x1, y0, y1)| value(ValueData::Curve(Curve::Rectangle(Rectangle {
                    plane,
                    x: Domain::new(x0, x1),
                    y: Domain::new(y0, y1),
                })))
            ),
    ]
}

/// Arbitrary valid meshes: a fan of triangles over up to 9 vertices, plus
/// the empty mesh (booleans produce it — it must roundtrip).
fn mesh_value() -> impl Strategy<Value = Arc<HashedValue>> {
    prop_oneof![
        Just(value(ValueData::Mesh(
            Mesh::new(vec![], vec![]).expect("empty mesh is valid")
        ))),
        (3usize..9).prop_flat_map(|vertices| {
            let positions = proptest::collection::vec(finite_f64(), vertices * 3);
            positions.prop_map(move |positions| {
                let count = u32::try_from(vertices).unwrap();
                let indices: Vec<u32> = (1..count - 1).flat_map(|i| [0, i, i + 1]).collect();
                value(ValueData::Mesh(
                    Mesh::new(positions, indices).expect("fan indices are valid"),
                ))
            })
        }),
    ]
}

/// Values up to 3 list levels deep, with optional axes and holes.
fn any_value() -> impl Strategy<Value = Arc<HashedValue>> {
    leaf_value().prop_recursive(3, 24, 5, |inner| {
        (
            proptest::option::of("[a-z]{1,8}"),
            proptest::collection::vec(proptest::option::of(inner), 0..5),
        )
            .prop_map(|(axis, slots)| {
                value(ValueData::List(List {
                    axis: axis.map(|axis| Arc::from(axis.as_str())),
                    slots,
                }))
            })
    })
}

proptest! {
    // Store → fresh-open → load: identical hash AND payload for arbitrary
    // Merkle trees. Loads verify against the address, so a passing load is
    // a proven-intact value.
    #[test]
    fn roundtrip_any_value(input in any_value()) {
        let dir = tempfile::tempdir().unwrap();
        {
            let (store, _) = DiskStore::open(dir.path()).unwrap();
            store.store_value(&input).unwrap();
        }
        // Fresh instance: nothing in memory, everything from disk.
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        let loaded = store.load_value(&input.hash()).unwrap();
        prop_assert_eq!(loaded.hash(), input.hash());
        prop_assert_eq!(loaded.data(), input.data());
    }
}

#[test]
fn shared_elements_are_stored_once() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let shared = value(ValueData::Number(42.0));
    let a = value(ValueData::List(List {
        axis: None,
        slots: vec![Some(shared.clone()), Some(shared.clone())],
    }));
    let b = value(ValueData::List(List {
        axis: Some(Arc::from("parts")),
        slots: vec![Some(shared.clone())],
    }));
    store.store_value(&a).unwrap();
    store.store_value(&b).unwrap();
    // Blobs: shared element + two list spines = 3 entries (small values
    // live in the pack; the sharded files are for big blobs only).
    let mut files = 0;
    for entry in std::fs::read_dir(dir.path().join("values")).unwrap() {
        let shard_dir = entry.unwrap().path();
        if shard_dir.is_dir() {
            files += std::fs::read_dir(shard_dir).unwrap().count();
        }
    }
    assert_eq!(
        store.packed_values() + files,
        3,
        "content addressing dedupes shared children"
    );
}

#[test]
fn big_blobs_get_their_own_file_and_small_ones_pack() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let small = value(ValueData::Number(1.5));
    // Incompressible bytes past the pack threshold: a random-looking text.
    let mut big_text = String::new();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    while big_text.len() < PACK_MAX_BYTES * 2 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        write!(big_text, "{x:016x}").unwrap();
    }
    let big = value(ValueData::Text(Arc::from(big_text.as_str())));
    store.store_value(&small).unwrap();
    store.store_value(&big).unwrap();
    assert!(matches!(
        store.locate_value(&small.hash()),
        Some(BlobLocation::Packed { .. })
    ));
    assert!(matches!(
        store.locate_value(&big.hash()),
        Some(BlobLocation::File(_))
    ));
    assert!(store.contains_value(&small.hash()));
    assert!(store.contains_value(&big.hash()));
    // Both reload verified from a fresh open (the pack index is rebuilt
    // from the file, not remembered).
    drop(store);
    let (store, report) = DiskStore::open(dir.path()).unwrap();
    assert_eq!(report.packed_values, 1);
    assert_eq!(report.pack_recovery, None);
    assert_eq!(
        store.load_value(&small.hash()).unwrap().data(),
        small.data()
    );
    assert_eq!(store.load_value(&big.hash()).unwrap().data(), big.data());
}

#[test]
fn torn_pack_tail_is_truncated_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let first = value(ValueData::Number(1.0));
    let second = value(ValueData::Number(2.0));
    {
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        store.store_value(&first).unwrap();
        store.store_value(&second).unwrap();
    }
    // Tear the last frame: drop its final byte (a crash mid-append).
    let pack = dir.path().join("values").join("pack.bin");
    let mut bytes = std::fs::read(&pack).unwrap();
    bytes.pop();
    std::fs::write(&pack, &bytes).unwrap();
    let (store, report) = DiskStore::open(dir.path()).unwrap();
    assert_eq!(
        report.pack_recovery,
        Some(cicada_sched::LogRecovery::TornTail)
    );
    assert_eq!(report.packed_values, 1, "the whole frame survives");
    assert!(store.contains_value(&first.hash()));
    assert!(!store.contains_value(&second.hash()), "torn frame is gone");
    // Re-storing the torn value appends cleanly after the truncation point
    // and replays on the next open.
    store.store_value(&second).unwrap();
    drop(store);
    let (store, report) = DiskStore::open(dir.path()).unwrap();
    assert_eq!(report.pack_recovery, None);
    assert_eq!(report.packed_values, 2);
    assert_eq!(
        store.load_value(&second.hash()).unwrap().data(),
        second.data()
    );
}

#[test]
fn corrupted_blob_is_refused_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let input = value(ValueData::Text(Arc::from("precious bytes")));
    {
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        store.store_value(&input).unwrap();
    }
    // Flip the blob's content, wherever it lives (a small Text packs).
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let location = store.locate_value(&input.hash()).expect("stored");
    let BlobLocation::Packed { path, offset, len } = &location else {
        panic!("a small value lives in the pack, got {location:?}");
    };
    let mut bytes = std::fs::read(path).unwrap();
    let start = usize::try_from(*offset).unwrap();
    bytes[start + *len as usize - 1] ^= 0xFF;
    std::fs::write(path, bytes).unwrap();
    // Reopen so the flipped bytes are what gets read (no memory copy).
    drop(store);
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let error = store
        .load_value(&input.hash())
        .expect_err("corruption must refuse");
    assert!(
        matches!(
            error,
            StoreError::CorruptValue { .. } | StoreError::Decode { .. }
        ),
        "got {error}"
    );
    // Regression (adversarial review, stage 3): the failed load QUARANTINES
    // the bad bytes (a packed frame is forgotten; a file is moved aside),
    // so a re-store writes good bytes and the address heals — never "Ok
    // forever on store, error forever on load".
    assert!(!store.contains_value(&input.hash()), "bad bytes forgotten");
    store.store_value(&input).unwrap();
    let healed = store.load_value(&input.hash()).unwrap();
    assert_eq!(healed.data(), input.data());
    // ...and the healed frame wins on replay (later frames supersede).
    drop(store);
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let healed = store.load_value(&input.hash()).unwrap();
    assert_eq!(healed.data(), input.data());
}

#[test]
fn missing_value_is_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = DiskStore::open(dir.path()).unwrap();
    let absent = value(ValueData::Number(123.456)).hash();
    assert!(!store.contains_value(&absent));
    assert!(matches!(
        store.load_value(&absent),
        Err(StoreError::MissingValue { .. })
    ));
}

#[test]
fn memory_budget_evicts_but_disk_still_serves() {
    let dir = tempfile::tempdir().unwrap();
    // A tiny budget: every insert evicts the previous entry.
    let (store, _) = DiskStore::open_with_budget(dir.path(), 64).unwrap();
    let values: Vec<Arc<HashedValue>> = (0..20)
        .map(|i| value(ValueData::Number(f64::from(i))))
        .collect();
    for v in &values {
        store.store_value(v).unwrap();
    }
    for v in &values {
        let loaded = store.load_value(&v.hash()).unwrap();
        assert_eq!(loaded.hash(), v.hash(), "evicted entries reload from disk");
    }
}

#[test]
fn default_cache_dir_is_never_inside_the_project() {
    let project = tempfile::tempdir().unwrap();
    let dir = project_cache_dir(project.path()).unwrap();
    let canonical_project = std::fs::canonicalize(project.path()).unwrap();
    assert!(
        !dir.starts_with(&canonical_project),
        "cache dir {} must not live inside project {} (DECISIONS.md cache row)",
        dir.display(),
        canonical_project.display()
    );
    // Distinct projects get distinct stores.
    let other = tempfile::tempdir().unwrap();
    let other_dir = project_cache_dir(other.path()).unwrap();
    assert_ne!(dir, other_dir, "stores are keyed per project");
}
