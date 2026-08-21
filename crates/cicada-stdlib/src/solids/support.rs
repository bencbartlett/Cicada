//! Test helpers shared by the solid nodes' tests.
//!
//! Golden-hash inputs must stay transcendental-free (boxes and rectangle
//! prisms, never spheres or rotations): sin/cos differ in the last ulp
//! across platform libms, so a transcendental-fed golden would be
//! platform-dependent. Curved B-rep primitives (`sphere`, `cylinder`,
//! `cone`) therefore have NO committed golden — their determinism tests
//! assert run-to-run byte identity and the analytic volume instead.
//!
//! The Solid goldens are hashes of OCCT canonical bytes, committed **per
//! platform** (DECISIONS.md row 42): every constant here was blessed on
//! win-64 and reaches the test through [`platform_golden`], the one place
//! a second platform adds its own arm when the CI matrix shows it
//! disagreeing — never by loosening the comparison.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Polyline, Rectangle, Solid};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point};
use cicada_core::value::{HashedValue, ValueData};
use cicada_geom::solid::kernel_available;

pub(crate) fn config() -> ProjectConfig {
    ProjectConfig::default()
}

pub(crate) fn unit_square_profile() -> Closed<Curve> {
    Closed(Curve::Rectangle(Rectangle {
        plane: Plane::world_xy(),
        x: Domain::new(0.0, 1.0),
        y: Domain::new(0.0, 1.0),
    }))
}

/// A closed polyline at height `z`: `(x, y)` corners in order.
pub(crate) fn ring(corners: &[(f64, f64)], z: f64) -> Closed<Curve> {
    Closed(Curve::Polyline(Polyline {
        vertices: corners.iter().map(|&(x, y)| Point::new(x, y, z)).collect(),
        closed: true,
    }))
}

/// Frustum volume between homothetic sections: h/3 · (A₁ + A₂ + √(A₁A₂)).
pub(crate) fn frustum_volume(area: f64, scale: f64, height: f64) -> f64 {
    let top = area * scale * scale;
    height / 3.0 * (area + top + (area * top).sqrt())
}

/// A plane at `origin` with the world axes.
pub(crate) fn plane_at(x: f64, y: f64, z: f64) -> Plane {
    Plane {
        origin: Point::new(x, y, z),
        ..Plane::world_xy()
    }
}

/// The committed golden for THIS platform. Every Solid golden in the
/// stdlib was blessed on win-64 (2026-08-20, OCCT 7.8.1 conda-forge build
/// 103); when the CI matrix shows another OS disagreeing on a
/// transcendental-free solid, add a `#[cfg(target_os = "…")]` arm here
/// with that OS's hash (a documented per-OS golden, DECISIONS.md row 42)
/// rather than loosening any comparison.
pub(crate) fn platform_golden(win64: &'static str) -> &'static str {
    win64
}

/// The `HashedValue` hash of a Solid — what the scheduler, store and
/// display key on.
pub(crate) fn solid_hash(solid: &Solid) -> String {
    HashedValue::new(ValueData::Solid(solid.clone()))
        .unwrap()
        .hash()
        .to_hex()
}

/// The analytic-volume oracle for a node's output.
pub(crate) fn volume_of(solid: &Solid) -> f64 {
    cicada_geom::solid::volume(solid).unwrap().volume
}

/// World-aligned bounds of a solid.
pub(crate) fn bounds_of(solid: &Solid) -> (Point, Point) {
    cicada_geom::solid::bounds(solid).unwrap()
}

/// Run a Solid node in both worlds (docs/14: never a vacuous pass): with
/// the kernel, `Some(output)`; without it, the node must be red with the
/// typed `KernelUnavailable` text and the test gets `None` — asserting
/// the refusal rather than silently passing. Every Solid node's table test
/// starts here.
pub(crate) fn with_kernel<T>(node: impl FnOnce() -> T + std::panic::UnwindSafe) -> Option<T> {
    if kernel_available() {
        return Some(node());
    }
    let Err(payload) = std::panic::catch_unwind(node) else {
        panic!("without the kernel a Solid node must be red, never pass");
    };
    let message = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .expect("a message");
    assert!(
        message.contains("needs the OCCT kernel"),
        "the refusal names the kernel: {message}"
    );
    None
}

/// A world-aligned B-rep box with its min corner at `origin` and the given
/// positive extents (the booleans' test fixture).
pub(crate) fn brep_box(origin: [f64; 3], extents: [f64; 3]) -> Solid {
    cicada_geom::solid::box_in_plane(
        &plane_at(origin[0], origin[1], origin[2]),
        Domain::new(0.0, extents[0]),
        Domain::new(0.0, extents[1]),
        Domain::new(0.0, extents[2]),
        config().tol(),
    )
    .unwrap()
}

/// `a` and `b` agree to a relative `rel` (absolute below 1).
pub(crate) fn close_rel(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}
