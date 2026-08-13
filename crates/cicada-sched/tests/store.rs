//! Disk-store contracts (docs/12 §The store): content-addressed roundtrips
//! for every value kind (property-tested over nested Merkle trees),
//! hash-verified loads that refuse corruption loudly, and the
//! never-inside-the-project rule for the default cache location.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cicada_core::scalar::{Color, Domain, IndexMap};
use cicada_core::spatial::{Plane, Point, Vector, Xform};
use cicada_core::value::{HashedValue, List, ValueData};
use cicada_sched::{DiskStore, StoreError, project_cache_dir};
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
    // Blobs: shared element + two list spines = 3 files.
    let mut blobs = 0;
    for shard in std::fs::read_dir(dir.path().join("values")).unwrap() {
        blobs += std::fs::read_dir(shard.unwrap().path()).unwrap().count();
    }
    assert_eq!(blobs, 3, "content addressing dedupes shared children");
}

#[test]
fn corrupted_blob_is_refused_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let input = value(ValueData::Text(Arc::from("precious bytes")));
    {
        let (store, _) = DiskStore::open(dir.path()).unwrap();
        store.store_value(&input).unwrap();
    }
    // Flip the blob's content.
    let hex = input.hash().to_hex();
    let blob = dir
        .path()
        .join("values")
        .join(&hex[..2])
        .join(format!("{hex}.zst"));
    let mut bytes = std::fs::read(&blob).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&blob, bytes).unwrap();

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
    // the bad blob, so a re-store writes good bytes and the address heals —
    // never "Ok forever on store, error forever on load".
    assert!(!blob.exists(), "bad blob moved aside");
    store.store_value(&input).unwrap();
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
