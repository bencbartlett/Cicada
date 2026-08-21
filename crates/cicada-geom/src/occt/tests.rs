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
//! path shows up here first, both as the raw blake3 of the bytes (WP-A's
//! goldens, unchanged) and as the `HashedValue` hash of the `core::Solid`
//! built from them through the value-level API (WP-B). Set
//! `CICADA_OCCT_DUMP=<dir>` to also write the bytes to files (e.g. to
//! sha256 them against the probe's recorded dumps).

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::{Point, Vector};
use cicada_core::value::{HashedValue, ValueData};
use opencascade_sys::cicada as glue;
use rayon::prelude::*;

use super::*;
use crate::meshbuild::signed_volume;
use crate::solid;

const TOL: f64 = 1e-6;

fn deflection() -> Deflection {
    Deflection::new(0.01, 0.5).expect("valid")
}

/// blake3 of the canonical bytes, as WP-A's golden hashes are written.
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

const BOX_GOLDEN: &str = "e220198abe7f57ebfc21340ff4291859f9681d17c9c9ac60f2005e3d4c1aa9e9";
const PRISM_GOLDEN: &str = "1f6c4615803ed53852f4d6904f520d6d2b9e15f8a08ada9fecae636a5fc962b6";

fn probe_box() -> Handle {
    Handle::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).expect("box")
}

fn probe_rectangle() -> [Point; 4] {
    [
        Point::new(3.0, 7.0, -5.0),
        Point::new(7.0, 7.0, -5.0),
        Point::new(7.0, 13.0, -5.0),
        Point::new(3.0, 13.0, -5.0),
    ]
}

fn probe_prism() -> Handle {
    Handle::extrude_polygon(&probe_rectangle(), Vector::new(0.0, 0.0, 40.0), TOL).expect("prism")
}

/// The value-level box and prism (what a node would produce).
fn box_value() -> Solid {
    solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).expect("box")
}

fn prism_value() -> Solid {
    solid::extrude_polygon(&probe_rectangle(), Vector::new(0.0, 0.0, 40.0), TOL).expect("prism")
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
    assert_golden("box", &bytes, BOX_GOLDEN);
}

#[test]
fn golden_canonical_bytes_extruded_rectangle() {
    let bytes = probe_prism().canonical_bytes().expect("bytes");
    assert!(bytes.starts_with(CANONICAL_HEADER));
    assert_eq!(bytes.len(), 2303, "the probe's size for this prism");
    assert_golden("extrude", &bytes, PRISM_GOLDEN);
}

/// WP-B: the same bytes through the value-level path, and the value hash
/// the scheduler, store and display key on. The raw blake3 goldens are
/// WP-A's, unchanged — the core path adds nothing to the bytes; the
/// `HashedValue` goldens (kind tag 20 over the length-prefixed bytes) are
/// blessed via run-once here.
#[test]
fn golden_value_hashes_through_the_core_path() {
    let cases = [
        (
            "box",
            box_value(),
            BOX_GOLDEN,
            "c17f91abe11669178363650582426abbf0e2c8c23f5dd173689475f9763d142b",
        ),
        (
            "extrude",
            prism_value(),
            PRISM_GOLDEN,
            "a00fdcf81271793de1b46d9416d31e56245fe9e3cb6aabc2671db5f2702be727",
        ),
    ];
    for (name, value, raw_golden, value_golden) in cases {
        assert_eq!(
            blake3_hex(value.bytes()),
            raw_golden,
            "{name}: the value-level path must yield WP-A's bytes"
        );
        let sealed = HashedValue::new(ValueData::Solid(value.clone())).expect("sealed");
        assert_eq!(
            sealed.hash().to_hex(),
            value_golden,
            "{name}: HashedValue hash of the Solid moved — bless via run-once"
        );
        // And the hash is a pure function of the bytes: a second construction
        // and a value rebuilt from the bytes agree.
        let again = Solid::from_canonical_bytes(value.bytes().to_vec()).expect("bytes");
        assert_eq!(
            HashedValue::new(ValueData::Solid(again))
                .expect("sealed")
                .hash(),
            sealed.hash()
        );
    }
}

#[test]
fn canonical_bytes_round_trip_is_a_fixed_point() {
    for (name, handle) in [("box", probe_box()), ("extrude", probe_prism())] {
        let first = handle.canonical_bytes().expect("bytes");
        let reread = Handle::from_canonical_bytes(&first).expect("read back");
        let second = reread.canonical_bytes().expect("bytes again");
        assert_eq!(
            first, second,
            "{name}: serialize → deserialize → serialize is byte-identical"
        );
        let again = Handle::from_canonical_bytes(&second).expect("read back again");
        assert_eq!(again.canonical_bytes().expect("bytes"), first);
    }
}

#[test]
fn canonical_bytes_ignore_display_tessellation() {
    // The hazard the fork fixed: meshing flips the faces' Checked flag and
    // attaches triangulation; neither may reach the canonical bytes. At the
    // value level this is immediate — tessellation consumes an op-local
    // handle and the value's bytes are never touched — so the assertion is
    // on the bytes produced AFTER a tessellation of a handle read from them.
    let value = box_value();
    let before = value.bytes().to_vec();
    let tessellation = solid::tessellate(&value, deflection()).expect("mesh");
    assert_eq!(tessellation.mesh.0.triangle_count(), 12);
    assert_eq!(tessellation.faces, 6);
    let rebuilt = Handle::from_value(&value).expect("read");
    assert_eq!(
        rebuilt.canonical_bytes().expect("bytes"),
        before,
        "tessellation is not part of a solid's identity"
    );
}

#[test]
fn canonical_bytes_are_identical_across_constructions() {
    // Two independent constructions of the same box (no shared TShapes).
    assert_eq!(
        probe_box().canonical_bytes().expect("a"),
        probe_box().canonical_bytes().expect("b"),
        "same inputs, same bytes, in one process (the probe proved it across processes)"
    );
    assert_eq!(box_value(), box_value());
}

// ---------------------------------------------------------------------------
// Tessellation
// ---------------------------------------------------------------------------

#[test]
fn box_tessellates_to_a_welded_watertight_cube() {
    let Tessellation { mesh, faces } = probe_box().tessellate(deflection()).expect("mesh");
    let mesh = mesh.0;
    assert_eq!(faces, 6);
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
    // Deterministic buffers run to run, and identical through the value path.
    assert_eq!(
        mesh,
        probe_box().tessellate(deflection()).expect("mesh").mesh.0
    );
    assert_eq!(
        mesh,
        solid::tessellate(&box_value(), deflection())
            .expect("mesh")
            .mesh
            .0
    );
}

#[test]
fn the_deflection_floor_is_the_meshers_own() {
    // Necessary: below Precision::Confusion the mesher throws. Driven
    // through the raw glue, bypassing `Deflection`'s validation, so this
    // pins OCCT's behaviour — the reason the floor exists — and proves the
    // exception arrives as an error, never an abort.
    let block = probe_box();
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let error = glue::cicada_tessellate(&block.inner, 1e-12, 0.1, &mut positions, &mut indices)
        .expect_err("a linear deflection under 1e-7 must be refused by the kernel");
    assert!(
        error.what().contains("Standard_NumericError")
            && error.what().contains("invalid parameter value"),
        "{}",
        error.what()
    );
    // Sufficient: exactly the floor, for both parameters, meshes.
    let floor = Deflection::new(solid::MIN_LINEAR_DEFLECTION, solid::MIN_ANGULAR_DEFLECTION)
        .expect("the floor is admitted");
    let tessellation = probe_box()
        .tessellate(floor)
        .expect("the kernel accepts its own floor");
    assert_eq!(tessellation.faces, 6);
    assert_eq!(tessellation.mesh.0.triangle_count(), 12);
    // And the value-level path refuses below it BEFORE the kernel: the
    // error is `Deflection::new`'s, so `solid::tessellate` never runs.
    assert!(matches!(
        Deflection::new(solid::MIN_LINEAR_DEFLECTION / 10.0, 0.1),
        Err(GeomError::BadParameter {
            name: "linear_deflection",
            ..
        })
    ));
}

#[test]
fn display_deflection_tessellates_the_probe_solids() {
    // The server's deflection for a default project draws planar solids
    // with exactly their face triangles.
    let display = Deflection::display(&ProjectConfig::default());
    let hole = solid::difference(&box_value(), &prism_value()).expect("cut");
    let tessellation = solid::tessellate(&hole, display).expect("mesh");
    assert_eq!(tessellation.faces, 10);
    assert_eq!(tessellation.mesh.0.triangle_count(), 32);
    assert!((signed_volume(&tessellation.mesh.0) - 5280.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Boolean difference
// ---------------------------------------------------------------------------

#[test]
fn difference_volume_matches_the_analytic_value() {
    // 10 × 20 × 30 box minus a 4 × 6 prism piercing it along Z:
    // 6000 − 4 · 6 · 30 = 5280. Planar faces, so the tessellated volume is
    // exact up to floating point.
    let hole = probe_box().difference(probe_prism()).expect("cut");
    let bytes = hole.canonical_bytes().expect("bytes");
    // The probe measured 8,821 B for the COMPOUND BRepAlgoAPI_Cut returns;
    // the seam unwraps it to the one solid inside, 18 B less.
    assert_eq!(bytes.len(), 8803);
    let Tessellation { mesh, faces } = hole.tessellate(deflection()).expect("mesh");
    assert!(mesh.0.is_watertight());
    assert_eq!(faces, 10);
    assert_eq!(mesh.0.triangle_count(), 32, "the probe's count (10 faces)");
    let volume = signed_volume(&mesh.0);
    assert!((volume - 5280.0).abs() < 1e-6, "got {volume}");
    // The result is one solid with its own canonical bytes, stable and
    // re-readable like any other — and the value-level cut yields exactly
    // these bytes.
    let reread = Handle::from_canonical_bytes(&bytes).expect("read");
    assert_eq!(reread.canonical_bytes().expect("bytes"), bytes);
    let value = solid::difference(&box_value(), &prism_value()).expect("cut");
    assert_eq!(value.bytes(), &bytes[..]);
}

#[test]
fn a_warm_difference_equals_a_cold_one() {
    // The determinism the sharing model buys: operating on a value several
    // times, in any order, yields the same bytes as the first time — the
    // inputs are re-read from their bytes, so no earlier boolean's
    // tolerance updates and no earlier tessellation can leak in.
    let block = box_value();
    let prism = prism_value();
    let cold = solid::difference(&block, &prism).expect("cut");
    let _ = solid::tessellate(&block, deflection()).expect("mesh");
    let _ = solid::tessellate(&prism, deflection()).expect("mesh");
    let other_cutter = solid::extrude_polygon(
        &[
            Point::new(1.0, 1.0, -5.0),
            Point::new(2.0, 1.0, -5.0),
            Point::new(2.0, 2.0, -5.0),
            Point::new(1.0, 2.0, -5.0),
        ],
        Vector::new(0.0, 0.0, 40.0),
        TOL,
    )
    .expect("cutter");
    let _ = solid::difference(&block, &other_cutter).expect("other cut");
    let warm = solid::difference(&block, &prism).expect("cut again");
    assert_eq!(cold, warm);
    assert_eq!(block, box_value(), "the input value never changed");
}

#[test]
fn difference_that_splits_or_empties_the_solid_is_refused() {
    // A slab through the middle (y ∈ [9, 11]) leaves two solids.
    let slab = Handle::extrude_polygon(
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
    let error = probe_box()
        .difference(slab)
        .expect_err("two solids must be refused");
    assert!(
        matches!(&error, GeomError::Kernel { reason } if reason.contains("found 2")),
        "{error}"
    );
    // A cutter that swallows the block leaves nothing.
    let bigger =
        Handle::box_at(Point::new(-1.0, -1.0, -1.0), Vector::new(12.0, 22.0, 32.0)).expect("big");
    let error = probe_box()
        .difference(bigger)
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
            Handle::box_at(Point::origin(), extents),
            Err(GeomError::BadParameter { .. })
        ));
        assert!(matches!(
            solid::box_at(Point::origin(), extents),
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
fn every_clause_of_the_exception_boundary_is_active_in_this_build() {
    // The fork's trycatch hook has three clauses (Standard_Failure,
    // std::exception, catch (...)); `cicada_selftest_throw` throws one
    // exception of each kind. Before the catch (...) clause a thrown int
    // unwound into Rust and killed the process (0xC0000409, measured with
    // the clause removed) — the same failure mode the probe saw for
    // Standard_Failure. This pins the boundary for THIS build of the glue,
    // not for the header as read.
    let occt = glue::cicada_selftest_throw(0).expect_err("Standard_Failure must be Err");
    assert!(
        occt.what().starts_with("Standard_DomainError: "),
        "{}",
        occt.what()
    );
    let std_error = glue::cicada_selftest_throw(1).expect_err("std::exception must be Err");
    assert_eq!(std_error.what(), "cicada_selftest_throw: std::exception");
    let foreign =
        glue::cicada_selftest_throw(2).expect_err("a thrown non-exception value must be Err");
    assert_eq!(foreign.what(), "unknown C++ exception");
    glue::cicada_selftest_throw(3).expect("kind 3 returns normally");
}

#[test]
fn degenerate_profiles_are_refused_with_the_mesh_tier_errors() {
    let up = Vector::new(0.0, 0.0, 1.0);
    // Too few points.
    assert!(matches!(
        Handle::extrude_polygon(&[Point::origin(), Point::new(1.0, 0.0, 0.0)], up, TOL),
        Err(GeomError::NotSimple { .. })
    ));
    // Collinear: OCCT would build a zero-volume solid; we refuse first.
    assert!(matches!(
        Handle::extrude_polygon(
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
        Handle::extrude_polygon(
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
        Handle::extrude_polygon(
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
        Handle::extrude_polygon(&probe_rectangle(), Vector::new(1.0, 0.0, 0.0), TOL),
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
        let error = Handle::from_canonical_bytes(bytes).expect_err("garbage must fail");
        assert!(matches!(error, GeomError::Serialization { .. }), "{error}");
    }
    // A value core accepted (header only) is still garbage to the kernel:
    // the value-level operations refuse it the same way, never crash.
    let pseudo = Solid::from_canonical_bytes(CANONICAL_HEADER.to_vec()).expect("header");
    assert!(matches!(
        solid::tessellate(&pseudo, deflection()),
        Err(GeomError::Serialization { .. })
    ));
    assert!(matches!(
        solid::difference(&box_value(), &pseudo),
        Err(GeomError::Serialization { .. })
    ));
}

// ---------------------------------------------------------------------------
// The sharing model: ownership, threads
// ---------------------------------------------------------------------------

#[test]
fn handle_is_send_and_the_value_is_sync() {
    // `Handle` crosses threads (the scheduler hands nodes to workers) but is
    // never shared between them — the `!Sync` assertion beside the type in
    // `mod.rs` is compile-time (a `Sync` handle would make that `const`
    // block ambiguous and fail the build); this test pins the positive half.
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Handle>();
    assert_send::<Solid>();
    assert_sync::<Solid>();
}

#[test]
fn kernel_operations_consume_their_handles() {
    // The type enforces rule 2 of the sharing model: `difference` and
    // `tessellate` take `self`, so a handle that has been through a kernel
    // operation cannot be used again — it no longer exists. What survives
    // is the value, and a fresh handle from it is pristine.
    let block = probe_box();
    let golden = block.canonical_bytes().expect("bytes");
    let hole = block.difference(probe_prism()).expect("cut");
    // `block` is gone; rebuild from the value and compare.
    let rebuilt = Handle::from_value(&box_value()).expect("read");
    assert_eq!(rebuilt.canonical_bytes().expect("bytes"), golden);
    let _ = hole.tessellate(deflection()).expect("mesh");
    // `hole` is gone too; the value path reproduces its bytes.
    assert_eq!(
        solid::difference(&box_value(), &prism_value())
            .expect("cut")
            .bytes()
            .len(),
        8803
    );
}

#[test]
fn related_solids_are_safe_across_rayon_workers() {
    // The module docs' hazard, made hard: M related values — a base block,
    // M_CUTTERS prisms through it at different offsets, and the M_CUTTERS
    // differences (which in OCCT share the block's untouched faces with it
    // at the moment they are computed) — worked on by N rayon threads at
    // once, every task picking a value and an operation by index: re-
    // serialize it, tessellate it, or recompute a difference. Under WP-A's
    // process-wide lock this was serialized; under the sharing model it is
    // genuinely parallel, and sound because every operation reads its own
    // graph from bytes and consumes it. Goldens are computed single-
    // threaded first; every parallel result must equal them. No sleeps, no
    // timing: the assertions are on the results.
    const THREADS: usize = 8;
    const M_CUTTERS: usize = 6;
    const ROUNDS: usize = 40;

    let block = box_value();
    let cutters: Vec<Solid> = (0..M_CUTTERS)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let x = 1.0 + i as f64;
            solid::extrude_polygon(
                &[
                    Point::new(x, 2.0, -5.0),
                    Point::new(x + 0.5, 2.0, -5.0),
                    Point::new(x + 0.5, 17.0, -5.0),
                    Point::new(x, 17.0, -5.0),
                ],
                Vector::new(0.0, 0.0, 40.0),
                TOL,
            )
            .expect("cutter")
        })
        .collect();
    let holes: Vec<Solid> = cutters
        .iter()
        .map(|cutter| solid::difference(&block, cutter).expect("cut"))
        .collect();
    let values: Vec<&Solid> = std::iter::once(&block)
        .chain(cutters.iter())
        .chain(holes.iter())
        .collect();
    let golden_meshes: Vec<Tessellation> = values
        .iter()
        .map(|value| solid::tessellate(value, deflection()).expect("mesh"))
        .collect();

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .expect("pool");
    let tasks = values.len() * 3 * ROUNDS;
    let outcomes: Vec<Result<(), String>> = pool.install(|| {
        (0..tasks)
            .into_par_iter()
            .map(|task| {
                let index = (task / 3) % values.len();
                let value = values[index];
                match task % 3 {
                    0 => {
                        // Re-serialize: a fresh handle's bytes are the value's.
                        let handle = Handle::from_value(value).map_err(|e| e.to_string())?;
                        let bytes = handle.canonical_bytes().map_err(|e| e.to_string())?;
                        (bytes == value.bytes())
                            .then_some(())
                            .ok_or_else(|| format!("value {index}: bytes drifted"))
                    }
                    1 => {
                        let mesh =
                            solid::tessellate(value, deflection()).map_err(|e| e.to_string())?;
                        (mesh == golden_meshes[index])
                            .then_some(())
                            .ok_or_else(|| format!("value {index}: tessellation drifted"))
                    }
                    _ => {
                        let cutter = index % M_CUTTERS;
                        let hole = solid::difference(&block, &cutters[cutter])
                            .map_err(|e| e.to_string())?;
                        (hole == holes[cutter])
                            .then_some(())
                            .ok_or_else(|| format!("cutter {cutter}: difference drifted"))
                    }
                }
            })
            .collect()
    });
    let failures: Vec<&String> = outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(outcomes.len(), tasks);
    // And nothing moved: the inputs are the values they were.
    assert_eq!(block, box_value());
    assert_eq!(blake3_hex(block.bytes()), BOX_GOLDEN);
}

// ---------------------------------------------------------------------------
// Determinism of the bytes across heap states and threads
// ---------------------------------------------------------------------------

/// The richest solid the seam can build today: the block minus six
/// through-slots along Z (disjoint, y ∈ [2, 17]) minus a channel along X
/// (y ∈ [8, 9], z ∈ [5, 10]) that crosses every slot — every cut after the
/// first intersects edges and faces the previous cuts created, so the
/// boolean's sub-shape maps hold real work, and the result is one solid.
fn carved_block() -> Solid {
    let mut shape = box_value();
    for i in 0..6 {
        let x = 1.0 + f64::from(i);
        let slot = solid::extrude_polygon(
            &[
                Point::new(x, 2.0, -5.0),
                Point::new(x + 0.5, 2.0, -5.0),
                Point::new(x + 0.5, 17.0, -5.0),
                Point::new(x, 17.0, -5.0),
            ],
            Vector::new(0.0, 0.0, 40.0),
            TOL,
        )
        .expect("slot");
        shape = solid::difference(&shape, &slot).expect("cut");
    }
    let channel = solid::extrude_polygon(
        &[
            Point::new(-1.0, 8.0, 5.0),
            Point::new(-1.0, 9.0, 5.0),
            Point::new(-1.0, 9.0, 10.0),
            Point::new(-1.0, 8.0, 10.0),
        ],
        Vector::new(12.0, 0.0, 0.0),
        TOL,
    )
    .expect("channel");
    solid::difference(&shape, &channel).expect("channel cut")
}

/// Deterministic heap churn: allocate and free blocks of LCG-chosen sizes
/// so the allocator hands OCCT different addresses on every call. OCCT's
/// `TopTools_ShapeMapHasher` hashes `TShape` ADDRESSES, so if any boolean
/// or mesher output depended on map iteration order, differing heap states
/// are exactly what would expose it.
fn churn_heap(seed: u64, rounds: usize) -> usize {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut kept: Vec<Vec<u8>> = Vec::new();
    let mut total = 0;
    for round in 0..rounds {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_possible_truncation)]
        let size = 16 + (state >> 33) as usize % 4096;
        total += size;
        kept.push(vec![0xA5; size]);
        if round % 3 == 0 {
            // Free an older block so holes open up mid-sequence.
            let victim = (state >> 17) as usize % kept.len();
            kept.swap_remove(victim);
        }
    }
    // `kept` drops here: the holes it leaves are the next operation's heap.
    total
}

#[test]
fn canonical_bytes_do_not_depend_on_heap_state_or_thread() {
    // The UNVERIFIED question of WP-B's review: OCCT's booleans and mesher
    // iterate maps keyed by TShape address — is the canonical serialization
    // (and the tessellation) a pure function of the inputs, or does it
    // follow the heap? Evidence here: the carved block (seven cuts, each
    // intersecting the last) computed cold, after deterministic churn of
    // several different seeds, and on N threads at once (each worker under
    // its own churn, so the addresses differ per thread too) — every result
    // byte-identical to the first, and every tessellation equal.
    const THREADS: usize = 8;
    const REPEATS: usize = 24;

    let golden = carved_block();
    let golden_mesh = solid::tessellate(&golden, deflection()).expect("mesh");
    assert_eq!(
        golden_mesh.faces, 58,
        "the block's 6 faces + 6 slots × 4 walls (the 12 x-walls the channel pierces \
         keep one face each, with a hole) + the channel's 4 walls, each split into \
         7 pieces by the 6 slots it crosses"
    );
    assert!(golden_mesh.mesh.0.is_watertight());

    // Serially, under different heap states.
    for seed in 1..=5u64 {
        let _ = churn_heap(seed, 2_000);
        let again = carved_block();
        assert_eq!(
            again.bytes(),
            golden.bytes(),
            "seed {seed}: the carved block's bytes followed the heap"
        );
        let _ = churn_heap(seed * 7, 500);
        assert_eq!(
            solid::tessellate(&again, deflection()).expect("mesh"),
            golden_mesh,
            "seed {seed}: the tessellation followed the heap"
        );
    }

    // In parallel, every worker churning differently between operations.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .expect("pool");
    let outcomes: Vec<Result<(), String>> = pool.install(|| {
        (0..REPEATS)
            .into_par_iter()
            .map(|repeat| {
                let seed = 100 + u64::try_from(repeat).map_err(|e| e.to_string())?;
                let _ = churn_heap(seed, 1_000 + repeat * 37);
                let carved = carved_block();
                if carved.bytes() != golden.bytes() {
                    return Err(format!("repeat {repeat}: bytes drifted"));
                }
                let _ = churn_heap(seed ^ 0xFF, 300);
                let mesh = solid::tessellate(&carved, deflection()).map_err(|e| e.to_string())?;
                (mesh == golden_mesh)
                    .then_some(())
                    .ok_or_else(|| format!("repeat {repeat}: tessellation drifted"))
            })
            .collect()
    });
    let failures: Vec<&String> = outcomes.iter().filter_map(|o| o.as_ref().err()).collect();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(outcomes.len(), REPEATS);
}

// ---------------------------------------------------------------------------
// weld(): the pure-Rust half of tessellate, on synthetic per-face buffers
// ---------------------------------------------------------------------------

/// A unit cube the way OCCT hands it over: 6 faces × 4 nodes (24 positions,
/// no sharing) and 6 × 2 triangles, counter-clockwise seen from outside.
fn unwelded_cube() -> (Vec<f64>, Vec<u32>) {
    // (origin, u, v) per face; quad = o, o+u, o+u+v, o+v; outward = u × v.
    let faces: [([f64; 3], [f64; 3], [f64; 3]); 6] = [
        ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]), // z = 0, normal −z
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // z = 1, normal +z
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]), // y = 0, normal −y
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]), // y = 1, normal +y
        ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), // x = 0, normal −x
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]), // x = 1, normal +x
    ];
    let mut positions = Vec::with_capacity(24 * 3);
    let mut indices = Vec::with_capacity(12 * 3);
    for (face, (o, u, v)) in faces.iter().enumerate() {
        let base = u32::try_from(face * 4).expect("small");
        for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            for axis in 0..3 {
                positions.push(o[axis] + corner[0] * u[axis] + corner[1] * v[axis]);
            }
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, indices)
}

#[test]
fn weld_merges_per_face_nodes_into_a_watertight_mesh() {
    let (positions, indices) = unwelded_cube();
    let mesh = weld(&positions, &indices).expect("closed").0;
    assert_eq!(mesh.vertex_count(), 8, "24 per-face nodes → 8 corners");
    assert_eq!(mesh.triangle_count(), 12);
    assert!(mesh.is_watertight());
    assert!((signed_volume(&mesh) - 1.0).abs() < 1e-12);
}

#[test]
fn weld_refuses_an_open_shell() {
    // Eleven of the twelve triangles: welded, consistent, but not closed.
    // Without the is_watertight check this would come back as a leaky
    // `Watertight<Mesh>` — the refusal IS the contract.
    let (positions, mut indices) = unwelded_cube();
    indices.truncate(11 * 3);
    let error = weld(&positions, &indices).expect_err("an open shell must be refused");
    assert!(
        matches!(&error, GeomError::NotWatertight { reason } if reason.contains("not closed")),
        "{error}"
    );
}

#[test]
fn weld_drops_triangles_that_collapse_onto_one_vertex() {
    // A sliver whose two nodes sit on the same position (a shared edge's
    // duplicate, as meshers produce them): distinct indices before welding,
    // a repeated index after. Kept, it would fail core's Mesh::new
    // (DegenerateTriangle) or break closure; weld drops it and the rest is
    // the same watertight cube.
    let (mut positions, mut indices) = unwelded_cube();
    let duplicate_of_zero = u32::try_from(positions.len() / 3).expect("small");
    positions.extend_from_slice(&[0.0, 0.0, 0.0]); // same point as node 0
    indices.extend_from_slice(&[0, duplicate_of_zero, 1]);
    let mesh = weld(&positions, &indices).expect("the sliver is dropped").0;
    assert_eq!(mesh.triangle_count(), 12);
    assert_eq!(mesh.vertex_count(), 8);
    assert!(mesh.is_watertight());
}

#[test]
fn weld_treats_negative_zero_as_zero() {
    // One face's zeros written as -0.0 (OCCT's transforms produce them):
    // bit-different, same point; they must weld or the cube leaks.
    let (mut positions, indices) = unwelded_cube();
    for value in positions.iter_mut().take(12) {
        if *value == 0.0 {
            *value = -0.0;
        }
    }
    assert!(positions.iter().any(|v| v.to_bits() == (-0.0f64).to_bits()));
    let mesh = weld(&positions, &indices).expect("welds").0;
    assert_eq!(mesh.vertex_count(), 8);
    assert!(mesh.is_watertight());
    assert!(
        mesh.positions()
            .iter()
            .all(|v| v.to_bits() != (-0.0f64).to_bits()),
        "the welded positions are canonical (+0.0)"
    );
}

#[test]
fn weld_refuses_ragged_and_non_finite_buffers() {
    let (positions, indices) = unwelded_cube();
    assert!(matches!(
        weld(&positions[..positions.len() - 1], &indices),
        Err(GeomError::Kernel { .. })
    ));
    assert!(matches!(
        weld(&positions, &indices[..indices.len() - 1]),
        Err(GeomError::Kernel { .. })
    ));
    let mut poisoned = positions.clone();
    poisoned[4] = f64::NAN;
    assert!(matches!(
        weld(&poisoned, &indices),
        Err(GeomError::Kernel { .. })
    ));
    let mut out_of_range = indices;
    out_of_range[0] = 24;
    assert!(matches!(
        weld(&positions, &out_of_range),
        Err(GeomError::Kernel { .. })
    ));
}
