//! The value model (docs/12 §Values): immutable values, blake3-hashed at
//! construction, Merkle lists with named axes and slot-preserving `Optional`
//! elements.
//!
//! Invariants enforced by [`HashedValue::new`], the only door in:
//! - **NaN is refused** — loud error, never a poisoned cache key.
//! - **`-0.0` is canonicalized to `0.0`** — no sign-of-zero ambiguity.
//! - The hash is computed exactly once, over the canonicalized bytes.
//!
//! A raw [`ValueData`] can hold anything; a [`HashedValue`] cannot. Only
//! hashed values circulate (wires, caches, lists).

use std::sync::Arc;

use crate::hash::{KindTag, ValueHash, ValueHasher};
use crate::scalar::{Color, Domain, IndexMap};
use crate::spatial::{Plane, Point, Vector, Xform};

/// The payload of a value, one variant per kind (docs/08 §Core value model;
/// geometry kinds join from `cicada-geom` in stage 4).
#[derive(Debug, Clone, PartialEq)]
pub enum ValueData {
    /// f64 scalar.
    Number(f64),
    /// i64 scalar.
    Integer(i64),
    /// Boolean scalar.
    Boolean(bool),
    /// Immutable text.
    Text(Arc<str>),
    /// Linear RGBA.
    Color(Color),
    /// 1-D interval.
    Domain(Domain),
    /// Element provenance indices.
    IndexMap(IndexMap),
    /// Position.
    Point(Point),
    /// Displacement.
    Vector(Vector),
    /// Oriented frame.
    Plane(Plane),
    /// Affine transform.
    Xform(Xform),
    /// A list of already-hashed elements (Merkle node), possibly with a
    /// named axis and absent (`Optional`) slots.
    List(List),
    /// The absent case of a standalone optional value (`T?` off a wire).
    /// Inside lists, absence is a `None` slot instead.
    Nothing,
}

impl ValueData {
    /// The kind's catalog name (`Number`, `List`, …) — error-message
    /// currency for marshalling and scheduler diagnostics.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Number(_) => "Number",
            Self::Integer(_) => "Integer",
            Self::Boolean(_) => "Boolean",
            Self::Text(_) => "Text",
            Self::Color(_) => "Color",
            Self::Domain(_) => "Domain",
            Self::IndexMap(_) => "IndexMap",
            Self::Point(_) => "Point",
            Self::Vector(_) => "Vector",
            Self::Plane(_) => "Plane",
            Self::Xform(_) => "Xform",
            Self::List(_) => "List",
            Self::Nothing => "Nothing",
        }
    }
}

/// A list level: optional axis name + slots. Slots hold already-constructed
/// [`HashedValue`]s, so the list hash is a hash of hashes — changing one
/// element re-hashes one leaf plus the spine, never the world.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    /// Named axis (`parts: Solid` — docs/09); `None` for anonymous lists.
    pub axis: Option<Arc<str>>,
    /// Elements; `None` is an absent `Optional` slot (slot-preserving,
    /// docs/08 rule 6).
    pub slots: Vec<Option<Arc<HashedValue>>>,
}

/// Where a NaN was found, for the loud refusal message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NanLocation {
    /// Kind of the offending value, e.g. `Point`.
    pub kind: &'static str,
    /// Component within the value, e.g. `y`.
    pub component: &'static str,
}

/// Value construction errors. Failing loudly here is the design: a NaN that
/// reached a cache key would poison equality downstream (docs/12).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// NaN refused at construction (docs/12: loud refusal; the scheduler
    /// attaches the producing node).
    #[error("NaN refused at value construction: {} component `{}` is NaN", .0.kind, .0.component)]
    NanRefused(NanLocation),
}

/// An immutable value with its content hash — the only value form that
/// circulates. Equality and hashing are by content hash.
#[derive(Debug)]
pub struct HashedValue {
    data: ValueData,
    hash: ValueHash,
}

impl HashedValue {
    /// Canonicalize, validate, hash, seal. The ONLY constructor.
    ///
    /// # Errors
    ///
    /// [`ValueError::NanRefused`] if any float component is NaN.
    pub fn new(mut data: ValueData) -> Result<Arc<Self>, ValueError> {
        canonicalize(&mut data)?;
        let hash = hash_data(&data);
        Ok(Arc::new(Self { data, hash }))
    }

    /// The payload.
    #[must_use]
    pub fn data(&self) -> &ValueData {
        &self.data
    }

    /// The content hash, computed at construction.
    #[must_use]
    pub fn hash(&self) -> ValueHash {
        self.hash
    }
}

impl PartialEq for HashedValue {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Eq for HashedValue {}

impl std::hash::Hash for HashedValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.hash.as_bytes().hash(state);
    }
}

/// Canonicalize and refuse NaN, recursively: `-0.0` → `0.0` in every float
/// slot, and a list slot holding a sealed `Nothing` value folds to a `None`
/// slot — two spellings of "absent" must not hash differently (same reason
/// `-0.0` collapses). List elements are otherwise already-sealed
/// `HashedValue`s and need no revisit.
fn canonicalize(data: &mut ValueData) -> Result<(), ValueError> {
    match data {
        ValueData::Number(x) => canon_f64(x, "Number", "value"),
        ValueData::List(list) => {
            for slot in &mut list.slots {
                if let Some(element) = slot
                    && matches!(element.data(), ValueData::Nothing)
                {
                    *slot = None;
                }
            }
            Ok(())
        }
        ValueData::Integer(_)
        | ValueData::Boolean(_)
        | ValueData::Text(_)
        | ValueData::IndexMap(_)
        | ValueData::Nothing => Ok(()),
        ValueData::Color(c) => {
            canon_f64(&mut c.r, "Color", "r")?;
            canon_f64(&mut c.g, "Color", "g")?;
            canon_f64(&mut c.b, "Color", "b")?;
            canon_f64(&mut c.a, "Color", "a")
        }
        ValueData::Domain(d) => {
            canon_f64(&mut d.start, "Domain", "start")?;
            canon_f64(&mut d.end, "Domain", "end")
        }
        ValueData::Point(p) => canon_vec3(&mut p.0, "Point"),
        ValueData::Vector(v) => canon_vec3(&mut v.0, "Vector"),
        ValueData::Plane(p) => {
            canon_vec3(&mut p.origin.0, "Plane")?;
            canon_vec3(&mut p.x.0, "Plane")?;
            canon_vec3(&mut p.y.0, "Plane")
        }
        ValueData::Xform(x) => {
            let m = &mut x.0.matrix3;
            canon_vec3(&mut m.x_axis, "Xform")?;
            canon_vec3(&mut m.y_axis, "Xform")?;
            canon_vec3(&mut m.z_axis, "Xform")?;
            canon_vec3(&mut x.0.translation, "Xform")
        }
    }
}

fn canon_f64(x: &mut f64, kind: &'static str, component: &'static str) -> Result<(), ValueError> {
    if x.is_nan() {
        return Err(ValueError::NanRefused(NanLocation { kind, component }));
    }
    if *x == 0.0 {
        *x = 0.0; // collapses -0.0; comparison already treats them equal
    }
    Ok(())
}

fn canon_vec3(v: &mut glam::DVec3, kind: &'static str) -> Result<(), ValueError> {
    canon_f64(&mut v.x, kind, "x")?;
    canon_f64(&mut v.y, kind, "y")?;
    canon_f64(&mut v.z, kind, "z")
}

/// Hash canonicalized data. Private: reachable only via [`HashedValue::new`].
fn hash_data(data: &ValueData) -> ValueHash {
    match data {
        ValueData::Number(x) => ValueHasher::new(KindTag::Number).f64(*x).finish(),
        ValueData::Integer(x) => ValueHasher::new(KindTag::Integer).i64(*x).finish(),
        ValueData::Boolean(b) => ValueHasher::new(KindTag::Boolean)
            .byte(u8::from(*b))
            .finish(),
        ValueData::Text(s) => ValueHasher::new(KindTag::Text).bytes(s.as_bytes()).finish(),
        ValueData::Color(c) => ValueHasher::new(KindTag::Color)
            .f64(c.r)
            .f64(c.g)
            .f64(c.b)
            .f64(c.a)
            .finish(),
        ValueData::Domain(d) => ValueHasher::new(KindTag::Domain)
            .f64(d.start)
            .f64(d.end)
            .finish(),
        ValueData::IndexMap(m) => {
            let mut hasher = ValueHasher::new(KindTag::IndexMap).u64(m.0.len() as u64);
            for &i in &m.0 {
                hasher = hasher.u64(i);
            }
            hasher.finish()
        }
        ValueData::Point(p) => ValueHasher::new(KindTag::Point)
            .f64(p.0.x)
            .f64(p.0.y)
            .f64(p.0.z)
            .finish(),
        ValueData::Vector(v) => ValueHasher::new(KindTag::Vector)
            .f64(v.0.x)
            .f64(v.0.y)
            .f64(v.0.z)
            .finish(),
        ValueData::Plane(p) => {
            let mut hasher = ValueHasher::new(KindTag::Plane);
            for v in [&p.origin.0, &p.x.0, &p.y.0] {
                hasher = hasher.f64(v.x).f64(v.y).f64(v.z);
            }
            hasher.finish()
        }
        ValueData::Xform(x) => {
            let mut hasher = ValueHasher::new(KindTag::Xform);
            for c in x.coefficients() {
                hasher = hasher.f64(c);
            }
            hasher.finish()
        }
        ValueData::List(list) => {
            // Merkle: axis name, then per slot presence + child hash
            // (docs/12: "a list's hash is the hash of its element hashes
            // (plus axis name); an Optional slot hashes its presence").
            let mut hasher = ValueHasher::new(KindTag::List);
            hasher = match &list.axis {
                Some(name) => hasher.byte(1).bytes(name.as_bytes()),
                None => hasher.byte(0),
            };
            hasher = hasher.u64(list.slots.len() as u64);
            for slot in &list.slots {
                hasher = match slot {
                    Some(element) => hasher.byte(1).child(&element.hash()),
                    None => hasher.byte(0),
                };
            }
            hasher.finish()
        }
        ValueData::Nothing => ValueHasher::new(KindTag::Nothing).finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(data: ValueData) -> Arc<HashedValue> {
        HashedValue::new(data).expect("valid value")
    }

    // Golden hashes: the cross-run / cross-platform determinism contract
    // (stage-1 DoD). Blessed by running once and copying the actual, as the
    // repo's blessed path prescribes. If one of these changes, the hash
    // FORMAT changed — bump HASH_FORMAT_VERSION and explain in the commit.
    #[test]
    fn golden_hashes() {
        let cases: &[(ValueData, &str)] = &[
            (
                ValueData::Number(1.5),
                "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03",
            ),
            (
                ValueData::Integer(-7),
                "60ced3195113544317bb194858b019f3990038084598e7587e30293d5cf50c41",
            ),
            (
                ValueData::Boolean(true),
                "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552",
            ),
            (
                ValueData::Text(Arc::from("cicada")),
                "73f5bdd62a60e1e85c40115d5eeed5b25f0c15b57b7141cae00b6ce70eccdf6a",
            ),
            (
                ValueData::Point(Point::new(1.0, 2.0, 3.0)),
                "1a6f8073cd8ceb247b753adbb96e270c282cc660b09bb99c4719b64d687b1ca2",
            ),
            (
                ValueData::Domain(Domain::new(0.0, 1.0)),
                "dba4d84fdff1c8eb2e98a3f45c8e429fd320dc06d133f27120f7a5fd7c10e0f4",
            ),
            (
                ValueData::Nothing,
                "bf22873ecb8e2fb471f984bef5cfaec7f97bff333191216b785a81cf9e1393a0",
            ),
            (
                ValueData::Color(Color::new(0.25, 0.5, 0.75, 1.0)),
                "a827e29d36d749c49c4ce8c9fbe83202132e4c20011686dde235a371427b61fc",
            ),
            (
                ValueData::Vector(Vector::new(-1.0, 0.5, 2.0)),
                "f22da081108fafa5248bb5b16702dd08e3b8466b35fdcfee9a707af874f0d724",
            ),
            (
                ValueData::Plane(Plane {
                    origin: Point::new(1.0, 2.0, 3.0),
                    x: Vector::new(1.0, 0.0, 0.0),
                    y: Vector::new(0.0, 1.0, 0.0),
                }),
                "a993a09358dc40debbe59d274bf97d97d4f4eab2041d11323965622f27329834",
            ),
            (
                ValueData::Xform(Xform::identity()),
                "74ca334b84cb50862ab9178c38a954a1479f8565a3ec4cec0d81510a31228227",
            ),
            (
                ValueData::IndexMap(IndexMap(vec![2, 0, 1])),
                "da032ff31d7eb1320fd5efa03029b8d591987de81deea391daf16cd9bfaf309f",
            ),
        ];
        // Report every drift at once — a format change moves all of them.
        let drifted: Vec<String> = cases
            .iter()
            .filter_map(|(data, want)| {
                let got = value(data.clone()).hash().to_hex();
                (&got != want).then(|| format!("{data:?}: got {got}"))
            })
            .collect();
        assert!(
            drifted.is_empty(),
            "hash format drifted:\n{}",
            drifted.join("\n")
        );
    }

    // Locks the full Merkle encoding in one case: named axis, a present
    // slot, and a None hole (axis presence byte, length-prefixed name,
    // count, per-slot presence + child hashes).
    #[test]
    fn golden_list_hash_with_axis_and_hole() {
        let list = value(ValueData::List(List {
            axis: Some(Arc::from("parts")),
            slots: vec![Some(value(ValueData::Number(1.5))), None],
        }));
        assert_eq!(
            list.hash().to_hex(),
            "3d0dae4fb7e6669af34884e4f2e14a63e6541d69d22bc565c0c6cf892c0078b7"
        );
    }

    #[test]
    fn negative_zero_canonicalizes_to_positive_zero() {
        let neg = value(ValueData::Number(-0.0));
        let pos = value(ValueData::Number(0.0));
        assert_eq!(neg.hash(), pos.hash());
        // The stored payload is canonical too, not just the hash.
        let ValueData::Number(x) = *neg.data() else {
            panic!("wrong variant")
        };
        assert!(x.is_sign_positive());
    }

    #[test]
    fn nan_is_refused_everywhere_a_float_lives() {
        let nan = f64::NAN;
        let cases: Vec<ValueData> = vec![
            ValueData::Number(nan),
            ValueData::Color(Color::new(0.0, nan, 0.0, 1.0)),
            ValueData::Domain(Domain::new(nan, 1.0)),
            ValueData::Point(Point::new(0.0, nan, 0.0)),
            ValueData::Vector(Vector::new(nan, 0.0, 0.0)),
            ValueData::Plane(Plane {
                origin: Point::new(0.0, 0.0, 0.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, nan, 0.0),
            }),
            {
                let mut in_matrix = Xform::identity();
                in_matrix.0.matrix3.y_axis.z = nan;
                ValueData::Xform(in_matrix)
            },
            {
                let mut in_translation = Xform::identity();
                in_translation.0.translation.x = nan;
                ValueData::Xform(in_translation)
            },
        ];
        for data in cases {
            assert!(
                matches!(
                    HashedValue::new(data.clone()),
                    Err(ValueError::NanRefused(_))
                ),
                "NaN slipped through {data:?}"
            );
        }
    }

    #[test]
    fn infinity_is_allowed() {
        // Only NaN is refused (docs/12); infinities hash by bits.
        assert!(HashedValue::new(ValueData::Number(f64::INFINITY)).is_ok());
    }

    #[test]
    fn merkle_list_axis_and_presence_matter() {
        let one = value(ValueData::Number(1.0));
        let plain = value(ValueData::List(List {
            axis: None,
            slots: vec![Some(one.clone())],
        }));
        let named = value(ValueData::List(List {
            axis: Some(Arc::from("parts")),
            slots: vec![Some(one.clone())],
        }));
        let with_hole = value(ValueData::List(List {
            axis: None,
            slots: vec![Some(one), None],
        }));
        assert_ne!(plain.hash(), named.hash(), "axis name must hash");
        assert_ne!(plain.hash(), with_hole.hash(), "slot presence must hash");
    }

    #[test]
    fn merkle_list_hash_changes_with_one_element() {
        let make = |x: f64| {
            value(ValueData::List(List {
                axis: None,
                slots: vec![
                    Some(value(ValueData::Number(1.0))),
                    Some(value(ValueData::Number(x))),
                ],
            }))
        };
        assert_ne!(make(2.0).hash(), make(3.0).hash());
        assert_eq!(make(2.0).hash(), make(2.0).hash());
    }

    #[test]
    fn some_nothing_slot_canonicalizes_to_none_slot() {
        // Two spellings of "absent" must be one value (same reason -0.0
        // collapses): a slot holding a sealed Nothing folds to a None slot.
        let one = value(ValueData::Number(1.0));
        let with_none = value(ValueData::List(List {
            axis: None,
            slots: vec![Some(one.clone()), None],
        }));
        let with_some_nothing = value(ValueData::List(List {
            axis: None,
            slots: vec![Some(one), Some(value(ValueData::Nothing))],
        }));
        assert_eq!(with_none.hash(), with_some_nothing.hash());
        let ValueData::List(list) = with_some_nothing.data() else {
            panic!("wrong variant")
        };
        assert!(
            list.slots[1].is_none(),
            "payload canonicalized, not just hash"
        );
    }

    #[test]
    fn empty_list_distinct_from_nothing() {
        let empty = value(ValueData::List(List {
            axis: None,
            slots: vec![],
        }));
        let nothing = value(ValueData::Nothing);
        assert_ne!(empty.hash(), nothing.hash());
    }

    #[test]
    fn equality_is_by_hash() {
        let a = value(ValueData::Number(4.25));
        let b = value(ValueData::Number(4.25));
        assert_eq!(a, b);
        assert!(!Arc::ptr_eq(&a, &b), "distinct allocations, equal by hash");
    }
}
