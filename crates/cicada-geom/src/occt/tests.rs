//! The OCCT seam's contract, against the real kernel (feature `occt`;
//! needs `DEP_OCCT_ROOT` at build time and the OCCT shared libraries on the
//! loader path at run time — `tools/fetch_occt.py --print-env`).
//!
//! The golden hashes are blessed via run-once (docs/14 §Testing standards):
//! run the test, read the hash it prints on mismatch, paste it, and explain
//! the diff in the commit body. They pin the CANONICAL bytes of
//! transcendental-free solids — the probe's box (10 × 20 × 30 at the origin)
//! and its extruded 4 × 6 rectangle (at (3, 7, −5), 40 along Z) — so any
//! change in OCCT build, format version, flag normalization or construction
//! path shows up here first. Set `CICADA_OCCT_DUMP=<dir>` to also write the
//! bytes to files (e.g. to sha256 them against the probe's recorded dumps).

use cicada_core::spatial::{Point, Vector};
use opencascade_sys::cicada as glue;

use super::*;
use crate::meshbuild::signed_volume;

const TOL: f64 = 1e-6;
const DEFLECTION: f64 = 0.01;
const ANGULAR: f64 = 0.5;

/// blake3 of the canonical bytes, as the golden hashes are written.
fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn dump(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("CICADA_OCCT_DUMP") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, bytes)
            .unwrap_or_else(|error| panic!("dump {}: {error}", path.display()));
    }
}

fn probe_box() -> Solid {
    Solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).expect("box")
}

fn probe_rectangle() -> [Point; 4] {
    [
        Point::new(3.0, 7.0, -5.0),
        Point::new(7.0, 7.0, -5.0),
        Point::new(7.0, 13.0, -5.0),
        Point::new(3.0, 13.0, -5.0),
    ]
}

fn probe_prism() -> Solid {
    Solid::extrude_polygon(&probe_rectangle(), Vector::new(0.0, 0.0, 40.0), TOL).expect("prism")
}

fn assert_golden(name: &str, bytes: &[u8], expected: &str) {
    dump(&format!("{name}.bin"), bytes);
    let got = blake3_hex(bytes);
    assert_eq!(
        got,
        expected,
        "{name}: canonical-bytes blake3 moved — {} bytes; bless via run-once",
        bytes.len()
    );
}

// ---------------------------------------------------------------------------
// Canonical bytes: golden hashes, round trip, history-independence
// ---------------------------------------------------------------------------

#[test]
fn golden_canonical_bytes_box() {
    let bytes = probe_box().canonical_bytes().expect("bytes");
    assert!(bytes.starts_with(CANONICAL_HEADER));
    assert_eq!(bytes.len(), 4494, "the probe's size for this box");
    assert_golden(
        "box",
        &bytes,
        "e220198abe7f57ebfc21340ff4291859f9681d17c9c9ac60f2005e3d4c1aa9e9",
    );
}

#[test]
fn golden_canonical_bytes_extruded_rectangle() {
    let bytes = probe_prism().canonical_bytes().expect("bytes");
    assert!(bytes.starts_with(CANONICAL_HEADER));
    assert_eq!(bytes.len(), 2303, "the probe's size for this prism");
    assert_golden(
        "extrude",
        &bytes,
        "1f6c4615803ed53852f4d6904f520d6d2b9e15f8a08ada9fecae636a5fc962b6",
    );
}

#[test]
fn canonical_bytes_round_trip_is_a_fixed_point() {
    for (name, solid) in [("box", probe_box()), ("extrude", probe_prism())] {
        let first = solid.canonical_bytes().expect("bytes");
        let reread = Solid::from_canonical_bytes(&first).expect("read back");
        let second = reread.canonical_bytes().expect("bytes again");
        assert_eq!(
            first, second,
            "{name}: serialize → deserialize → serialize is byte-identical"
        );
        let again = Solid::from_canonical_bytes(&second).expect("read back again");
        assert_eq!(again.canonical_bytes().expect("bytes"), first);
    }
}

#[test]
fn canonical_bytes_ignore_display_tessellation() {
    // The hazard the fork fixed: meshing flips the faces' Checked flag and
    // attaches triangulation; neither may reach the canonical bytes.
    let solid = probe_box();
    let before = solid.canonical_bytes().expect("bytes");
    let mesh = solid.tessellate(DEFLECTION, ANGULAR).expect("mesh");
    assert_eq!(mesh.0.triangle_count(), 12);
    let after = solid.canonical_bytes().expect("bytes");
    assert_eq!(
        before, after,
        "tessellation is not part of a solid's identity"
    );
    // And equal solids reached via different histories hash equally: a
    // solid rebuilt from bytes is the same value as the fresh one.
    let rebuilt = Solid::from_canonical_bytes(&after).expect("read");
    assert_eq!(rebuilt.canonical_bytes().expect("bytes"), before);
}

#[test]
fn canonical_bytes_are_identical_across_constructions() {
    // Two independent constructions of the same box (no shared TShapes).
    assert_eq!(
        probe_box().canonical_bytes().expect("a"),
        probe_box().canonical_bytes().expect("b"),
        "same inputs, same bytes, in one process (the probe proved it across processes)"
    );
}

// ---------------------------------------------------------------------------
// Tessellation
// ---------------------------------------------------------------------------

#[test]
fn box_tessellates_to_a_welded_watertight_cube() {
    let mesh = probe_box().tessellate(DEFLECTION, ANGULAR).expect("mesh").0;
    assert_eq!(mesh.triangle_count(), 12, "6 faces × 2 triangles");
    assert_eq!(
        mesh.vertex_count(),
        8,
        "24 per-face nodes weld to the 8 corners"
    );
    assert!(mesh.is_watertight());
    let volume = signed_volume(&mesh);
    assert!(
        (volume - 6000.0).abs() < 1e-9,
        "outward orientation, exact planar volume: {volume}"
    );
    // Deterministic buffers run to run.
    assert_eq!(
        mesh,
        probe_box().tessellate(DEFLECTION, ANGULAR).expect("mesh").0
    );
}

#[test]
fn tessellation_rejects_bad_deflections() {
    let solid = probe_box();
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            solid.tessellate(bad, ANGULAR),
            Err(GeomError::BadParameter {
                name: "linear_deflection",
                ..
            })
        ));
        assert!(matches!(
            solid.tessellate(DEFLECTION, bad),
            Err(GeomError::BadParameter {
                name: "angular_deflection",
                ..
            })
        ));
    }
}

// ---------------------------------------------------------------------------
// Boolean difference
// ---------------------------------------------------------------------------

#[test]
fn difference_volume_matches_the_analytic_value() {
    // 10 × 20 × 30 box minus a 4 × 6 prism piercing it along Z:
    // 6000 − 4 · 6 · 30 = 5280. Planar faces, so the tessellated volume is
    // exact up to floating point.
    let hole = probe_box().difference(&probe_prism()).expect("cut");
    let mesh = hole.tessellate(DEFLECTION, ANGULAR).expect("mesh").0;
    assert!(mesh.is_watertight());
    assert_eq!(mesh.triangle_count(), 32, "the probe's count (10 faces)");
    let volume = signed_volume(&mesh);
    assert!((volume - 5280.0).abs() < 1e-6, "got {volume}");
    // The result is one solid with its own canonical bytes, stable and
    // re-readable like any other.
    let bytes = hole.canonical_bytes().expect("bytes");
    // The probe measured 8,821 B for the COMPOUND BRepAlgoAPI_Cut returns;
    // the seam unwraps it to the one solid inside, 18 B less.
    assert_eq!(bytes.len(), 8803);
    let reread = Solid::from_canonical_bytes(&bytes).expect("read");
    assert_eq!(reread.canonical_bytes().expect("bytes"), bytes);
}

#[test]
fn difference_that_splits_or_empties_the_solid_is_refused() {
    let block = probe_box();
    // A slab through the middle (y ∈ [9, 11]) leaves two solids.
    let slab = Solid::extrude_polygon(
        &[
            Point::new(-1.0, 9.0, -1.0),
            Point::new(11.0, 9.0, -1.0),
            Point::new(11.0, 11.0, -1.0),
            Point::new(-1.0, 11.0, -1.0),
        ],
        Vector::new(0.0, 0.0, 32.0),
        TOL,
    )
    .expect("slab");
    let error = block
        .difference(&slab)
        .expect_err("two solids must be refused");
    assert!(
        matches!(&error, GeomError::Kernel { reason } if reason.contains("found 2")),
        "{error}"
    );
    // A cutter that swallows the block leaves nothing.
    let bigger =
        Solid::box_at(Point::new(-1.0, -1.0, -1.0), Vector::new(12.0, 22.0, 32.0)).expect("big");
    let error = block
        .difference(&bigger)
        .expect_err("no solid must be refused");
    assert!(
        matches!(&error, GeomError::Kernel { reason } if reason.contains("found 0")),
        "{error}"
    );
}

// ---------------------------------------------------------------------------
// Failing and degenerate inputs are errors, never aborts
// ---------------------------------------------------------------------------

#[test]
fn degenerate_box_is_refused_before_the_kernel() {
    for extents in [
        Vector::new(0.0, 1.0, 1.0),
        Vector::new(1.0, -2.0, 1.0),
        Vector::new(1.0, 1.0, f64::NAN),
    ] {
        assert!(matches!(
            Solid::box_at(Point::origin(), extents),
            Err(GeomError::BadParameter { .. })
        ));
    }
}

#[test]
fn a_kernel_exception_arrives_as_an_error_not_an_abort() {
    // Straight through the bridge, bypassing the Rust-side validation: a
    // 0 × 0 × 0 box makes BRepPrim_GWedge::Shell throw Standard_DomainError
    // inside OCCT. Before the fork's trycatch hook this line terminated the
    // process with "Rust cannot catch foreign exceptions" (exit 0xC0000409,
    // probe `throw`). Now it is an Err carrying the OCCT exception type.
    let error = glue::cicada_make_box(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        .err()
        .expect("must fail");
    assert!(
        error.what().contains("Standard_DomainError"),
        "{}",
        error.what()
    );
}

#[test]
fn degenerate_profiles_are_refused_with_the_mesh_tier_errors() {
    let up = Vector::new(0.0, 0.0, 1.0);
    // Too few points.
    assert!(matches!(
        Solid::extrude_polygon(&[Point::origin(), Point::new(1.0, 0.0, 0.0)], up, TOL),
        Err(GeomError::NotSimple { .. })
    ));
    // Collinear: OCCT would build a zero-volume solid; we refuse first.
    assert!(matches!(
        Solid::extrude_polygon(
            &[
                Point::origin(),
                Point::new(1.0, 0.0, 0.0),
                Point::new(2.0, 0.0, 0.0)
            ],
            up,
            TOL
        ),
        Err(GeomError::DegenerateFrame { .. })
    ));
    // Non-planar.
    assert!(matches!(
        Solid::extrude_polygon(
            &[
                Point::origin(),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.5),
                Point::new(0.0, 1.0, 0.0)
            ],
            up,
            TOL
        ),
        Err(GeomError::NotPlanar { .. })
    ));
    // Self-intersecting with non-zero area (edges cross; the mesh tier's
    // post-validation catch — a symmetric bow tie would already fail the
    // Newell frame as zero-area).
    assert!(matches!(
        Solid::extrude_polygon(
            &[
                Point::origin(),
                Point::new(4.0, 0.0, 0.0),
                Point::new(1.0, 2.0, 0.0),
                Point::new(3.0, 2.0, 0.0)
            ],
            up,
            TOL
        ),
        Err(GeomError::NotSimple { .. })
    ));
    // Direction in the profile plane.
    assert!(matches!(
        Solid::extrude_polygon(&probe_rectangle(), Vector::new(1.0, 0.0, 0.0), TOL),
        Err(GeomError::BadParameter {
            name: "direction",
            ..
        })
    ));
}

#[test]
fn garbage_bytes_are_a_serialization_error() {
    for bytes in [
        &b""[..],
        b"not a brep",
        b"\nOpen CASCADE Topology V4, (c) Open Cascade\ntruncated",
    ] {
        let error = Solid::from_canonical_bytes(bytes).expect_err("garbage must fail");
        assert!(matches!(error, GeomError::Serialization { .. }), "{error}");
    }
}

#[test]
fn solid_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Solid>();
}
