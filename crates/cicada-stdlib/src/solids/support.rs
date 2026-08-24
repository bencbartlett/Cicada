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
/// transcendental-free solid, add that OS's row to [`PLATFORM_ARMS`]
/// (a documented per-OS golden, DECISIONS.md row 42) rather than
/// loosening any comparison.
///
/// The first matrix run with the kernel in every job (CI 32508005776,
/// 2026-08-21, macos-latest = arm64): linux-64 agrees with win-64 on all
/// ten Solid goldens, and nine of the ten agree with osx-arm64 byte for
/// byte — box, extrude, extrude to a point, the three booleans, bounding
/// box, deconstruct, tessellate; only
/// `loft` differs (`BRepOffsetAPI_ThruSections` builds ruled B-spline
/// surfaces whose control-point arithmetic rounds differently on that
/// build). Its macOS hash is the one that run printed — blessed from CI's
/// own output, never typed from a guess; a golden without an arm for the
/// running OS falls back to the win-64 value and, if it differs, fails
/// loudly with both hashes, which is how the next arm gets added.
///
/// The arms are a TABLE, not `cfg`-gated code: the per-PR CI lints on
/// Linux only, so a body that only macOS compiles is linted only by the
/// nightly matrix — the first arm, written as a one-armed `match`, was
/// `clippy::single_match` there for three nights (2026-08-22..24) while
/// every per-PR job stayed green. The lookup below is the same code on
/// every OS; only the data differs.
pub(crate) fn platform_golden(win64: &'static str) -> &'static str {
    PLATFORM_ARMS
        .iter()
        .find(|(blessed_on_win64, _)| *blessed_on_win64 == win64)
        .map_or(win64, |(_, this_os)| *this_os)
}

/// `(the win-64 hash, this OS's hash)` for every golden this OS disagrees
/// on. Empty where the OS agrees with win-64 on all of them (linux-64 does).
#[cfg(target_os = "macos")]
const PLATFORM_ARMS: &[(&str, &str)] = &[
    // loft_determinism_golden_hash — the ruled frustum between two
    // pentagonal rings 12 units apart.
    (
        "bf5a61c9a03e5e9add5fb41899d27618cc3205df556f611cb2cc229bf4a6a617",
        "958dddb63ef5d6411f98ddc7b67f8695c0b298dc7e188d9a2367a6c80458c463",
    ),
];
#[cfg(not(target_os = "macos"))]
const PLATFORM_ARMS: &[(&str, &str)] = &[];

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

/// The text every kernel refusal carries (`GeomError::KernelUnavailable`).
pub(crate) const KERNEL_REFUSAL: &str = "needs the OCCT kernel";

/// Run a Solid node in both worlds (docs/14: never a vacuous pass): with
/// the kernel, `Some(output)`; without it, the node must be red with the
/// typed `KernelUnavailable` text and the test gets `None` — asserting
/// the refusal rather than silently passing. Every Solid node's table test
/// starts here. The kernel-free world is real since the review closure of
/// 2026-08-21: `cargo test -p cicada-stdlib --no-default-features` (the
/// crate's own `occt` feature forwards to cicada-geom's), and a test's
/// Solid INPUTS come from [`fixture`] / [`brep_box`] so that without the
/// kernel the node under test is reached with a pseudo solid and the
/// refusal asserted here is the node's own.
pub(crate) fn with_kernel<T>(node: impl FnOnce() -> T + std::panic::UnwindSafe) -> Option<T> {
    if kernel_available() {
        return Some(node());
    }
    let Err(payload) = std::panic::catch_unwind(node) else {
        panic!("without the kernel a Solid node must be red, never pass");
    };
    let message = panic_text(&payload);
    assert!(
        message.contains(KERNEL_REFUSAL),
        "the refusal names the kernel: {message}"
    );
    None
}

/// Assert a node call is red in both worlds: with the kernel, for `reason`
/// (a substring of the node's message — the input or kernel refusal under
/// test); without it, with the typed kernel refusal. Returns the message
/// for further, kernel-world-only assertions. Never vacuous in either
/// world.
pub(crate) fn expect_red<T>(
    call: impl FnOnce() -> T + std::panic::UnwindSafe,
    reason: &str,
) -> String {
    let Err(payload) = std::panic::catch_unwind(call) else {
        panic!("the call must be red (expected: {reason})");
    };
    let message = panic_text(&payload);
    let expected = if kernel_available() {
        reason
    } else {
        KERNEL_REFUSAL
    };
    assert!(
        message.contains(expected),
        "expected `{expected}` in: {message}"
    );
    message
}

fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .expect("a message")
}

/// A value that IS a `Solid` to the value model — the canonical header and
/// nothing else — for the kernel-free world, where no real solid can be
/// built: a node handed it reaches its kernel call and refuses there,
/// typed. (With the kernel it is a serialization error, never a crash;
/// the tests never hand it to the kernel.)
pub(crate) fn pseudo_solid() -> Solid {
    Solid::from_canonical_bytes(cicada_core::geometry::SOLID_CANONICAL_HEADER.to_vec())
        .expect("the header is a valid value")
}

/// A test's Solid INPUT from a kernel constructor: the solid with the
/// kernel; the [`pseudo_solid`] without it (so the node under test, not the
/// fixture, is what refuses); any other failure is the fixture's bug.
pub(crate) fn fixture(result: Result<Solid, cicada_geom::GeomError>) -> Solid {
    match result {
        Ok(solid) => solid,
        Err(cicada_geom::GeomError::KernelUnavailable { .. }) => pseudo_solid(),
        Err(error) => panic!("fixture: {error}"),
    }
}

/// A world-aligned B-rep box with its min corner at `origin` and the given
/// positive extents (the booleans' test fixture); the [`pseudo_solid`]
/// without the kernel.
pub(crate) fn brep_box(origin: [f64; 3], extents: [f64; 3]) -> Solid {
    fixture(cicada_geom::solid::box_in_plane(
        &plane_at(origin[0], origin[1], origin[2]),
        Domain::new(0.0, extents[0]),
        Domain::new(0.0, extents[1]),
        Domain::new(0.0, extents[2]),
        config().tol(),
    ))
}

/// `a` and `b` agree to a relative `rel` (absolute below 1).
pub(crate) fn close_rel(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}
