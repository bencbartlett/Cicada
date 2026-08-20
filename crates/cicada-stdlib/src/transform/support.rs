//! Test helpers shared by the transform nodes' tests.
//!
//! Golden-hash inputs stay transcendental-free (docs/14): translation,
//! scale, axis-permuted orient, and a ZERO-angle rotation are pure
//! arithmetic (sin 0 = 0 and cos 0 = 1 are exact in every libm);
//! non-trivial rotation angles would make the hash platform-dependent
//! and are forbidden in goldens.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Point;
use cicada_core::value::{HashedValue, ValueData};

pub(crate) fn config() -> ProjectConfig {
    ProjectConfig::default()
}

pub(crate) fn point(x: f64, y: f64, z: f64) -> Transformable {
    Transformable::Point(Point::new(x, y, z))
}

pub(crate) fn expect_point(value: &Transformable) -> Point {
    match value {
        Transformable::Point(p) => *p,
        other => panic!("expected Point, got {other:?}"),
    }
}

pub(crate) fn expect_point_hash(value: &Transformable) -> String {
    let Transformable::Point(p) = value else {
        panic!("point stays a point")
    };
    HashedValue::new(ValueData::Point(*p))
        .unwrap()
        .hash()
        .to_hex()
}
