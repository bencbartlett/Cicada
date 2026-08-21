//! The node set's kernel operations (v0.1 item 3 WP-C), against the real
//! kernel: primitives, sweeps, booleans, measurement, topology readers,
//! transforms and STEP — every operation `glue.hxx` adds, exercised through
//! the value level (`crate::solid`) as the stdlib nodes call it. Analytic
//! volumes are the oracle (`solid::volume` and the tessellated
//! `signed_volume` agree with each other and with the formula); golden
//! hashes stay with the nodes (transcendental-free inputs only), except the
//! byte-identity claims made here: a `box_in_plane` in the world frame IS
//! `box_at`, an `extrude` of a rectangle IS `extrude_polygon`.

use std::f64::consts::{PI, TAU};

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Circle, Curve, Line, Polyline, Rectangle, Solid};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};

use super::*;
use crate::meshbuild::signed_volume;
use crate::solid::{self, Deflection, VolumeProperties};
use crate::transform::Similarity;

const TOL: f64 = 1e-6;
const TOL_ANGLE: f64 = 1e-9;

fn fine() -> Deflection {
    Deflection::new(0.001, 0.05).expect("valid")
}

fn xy_at(z: f64) -> Plane {
    Plane {
        origin: Point::new(0.0, 0.0, z),
        ..Plane::world_xy()
    }
}

fn square(side: f64, z: f64) -> Curve {
    Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(0.0, 0.0, z),
            Point::new(side, 0.0, z),
            Point::new(side, side, z),
            Point::new(0.0, side, z),
        ],
        closed: true,
    })
}

fn circle(radius: f64, plane: Plane) -> Curve {
    Curve::Circle(Circle { plane, radius })
}

fn volume_of(solid: &Solid) -> f64 {
    let VolumeProperties { volume, .. } = solid::volume(solid).expect("volume");
    // The mesher agrees with the integrator (to the chord error of a fine
    // tessellation): two independent readings of the same solid.
    let mesh = solid::tessellate(solid, fine()).expect("mesh").mesh.0;
    let by_mesh = signed_volume(&mesh);
    assert!(
        (by_mesh - volume).abs() <= 2e-3 * volume.abs().max(1.0),
        "integrator {volume} vs mesh {by_mesh}"
    );
    volume
}

fn assert_close(got: f64, want: f64, rel: f64) {
    assert!(
        (got - want).abs() <= rel * want.abs().max(1.0),
        "got {got}, want {want}"
    );
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

#[test]
fn box_in_the_world_frame_is_box_at() {
    let by_plane = solid::box_in_plane(
        &Plane::world_xy(),
        Domain::new(0.0, 10.0),
        Domain::new(0.0, 20.0),
        Domain::new(0.0, 30.0),
        TOL,
    )
    .expect("box");
    let by_corner = solid::box_at(Point::origin(), Vector::new(10.0, 20.0, 30.0)).expect("box");
    assert_eq!(by_plane, by_corner, "same construction path, same bytes");
    // Decreasing domains normalize; the minimum corner is the domains' start.
    let flipped = solid::box_in_plane(
        &Plane::world_xy(),
        Domain::new(10.0, 0.0),
        Domain::new(20.0, 0.0),
        Domain::new(30.0, 0.0),
        TOL,
    )
    .expect("box");
    assert_eq!(flipped, by_corner);
    let (min, max) = solid::bounds(&by_plane).expect("bounds");
    assert!(tol::coincident(min, Point::origin(), 1e-9));
    assert!(tol::coincident(max, Point::new(10.0, 20.0, 30.0), 1e-9));
}

#[test]
fn primitives_have_their_analytic_volumes() {
    let sphere = solid::sphere(&xy_at(1.0), 2.0, TOL).expect("sphere");
    assert_close(volume_of(&sphere), 4.0 / 3.0 * PI * 8.0, 1e-9);
    let VolumeProperties { centroid, .. } = solid::volume(&sphere).expect("props");
    assert!(tol::coincident(centroid, Point::new(0.0, 0.0, 1.0), 1e-9));

    let cylinder = solid::cylinder(&Plane::world_xy(), 1.5, 4.0, TOL).expect("cylinder");
    assert_close(volume_of(&cylinder), PI * 2.25 * 4.0, 1e-9);
    let (min, max) = solid::bounds(&cylinder).expect("bounds");
    assert!(tol::coincident(min, Point::new(-1.5, -1.5, 0.0), 1e-7));
    assert!(tol::coincident(max, Point::new(1.5, 1.5, 4.0), 1e-7));

    let cone = solid::cone(&Plane::world_xy(), 3.0, 6.0, TOL).expect("cone");
    assert_close(volume_of(&cone), PI * 9.0 * 6.0 / 3.0, 1e-9);
    let VolumeProperties { centroid, .. } = solid::volume(&cone).expect("props");
    assert!(tol::coincident(centroid, Point::new(0.0, 0.0, 1.5), 1e-9));
}

#[test]
fn primitives_refuse_degenerate_inputs_before_the_kernel() {
    assert!(matches!(
        solid::sphere(&Plane::world_xy(), 0.0, TOL),
        Err(GeomError::BadParameter { name: "radius", .. })
    ));
    assert!(matches!(
        solid::cylinder(&Plane::world_xy(), 1.0, -1.0, TOL),
        Err(GeomError::BadParameter { name: "height", .. })
    ));
    assert!(matches!(
        solid::cone(&Plane::world_xy(), f64::NAN, 1.0, TOL),
        Err(GeomError::BadParameter { name: "radius", .. })
    ));
    assert!(matches!(
        solid::box_in_plane(
            &Plane::world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(2.0, 2.0),
            Domain::new(0.0, 1.0),
            TOL
        ),
        Err(GeomError::BadParameter { name: "y", .. })
    ));
    let flat = Plane {
        y: Vector::new(2.0, 0.0, 0.0),
        ..Plane::world_xy()
    };
    assert!(matches!(
        solid::sphere(&flat, 1.0, TOL),
        Err(GeomError::DegenerateFrame { .. })
    ));
}

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

#[test]
fn extrude_of_a_rectangle_is_the_forks_prism() {
    let rectangle = Curve::Rectangle(Rectangle {
        plane: xy_at(-5.0),
        x: Domain::new(3.0, 7.0),
        y: Domain::new(7.0, 13.0),
    });
    let by_curve = solid::extrude(&rectangle, Vector::new(0.0, 0.0, 40.0), TOL).expect("prism");
    let by_polygon = solid::extrude_polygon(
        &[
            Point::new(3.0, 7.0, -5.0),
            Point::new(7.0, 7.0, -5.0),
            Point::new(7.0, 13.0, -5.0),
            Point::new(3.0, 13.0, -5.0),
        ],
        Vector::new(0.0, 0.0, 40.0),
        TOL,
    )
    .expect("prism");
    assert_eq!(by_curve, by_polygon);
    assert_close(volume_of(&by_curve), 4.0 * 6.0 * 40.0, 1e-9);
}

#[test]
fn extrude_of_a_circle_is_an_exact_cylinder() {
    let prism = solid::extrude(
        &circle(2.0, Plane::world_xy()),
        Vector::new(0.0, 0.0, 5.0),
        TOL,
    )
    .expect("cylinder");
    assert_close(volume_of(&prism), PI * 4.0 * 5.0, 1e-9);
    let (edges, vertices, faces) = solid::edges_and_vertices(&prism, fine()).expect("topology");
    assert_eq!(faces, 3, "two caps and one cylindrical face");
    assert!(
        edges
            .iter()
            .filter(|e| matches!(e, Curve::Circle(_)))
            .count()
            >= 2,
        "the caps' rims are exact circles: {edges:?}"
    );
    assert!(!vertices.is_empty());
    // An oblique extrusion keeps the base area × normal height (Cavalieri).
    let sheared =
        solid::extrude(&square(2.0, 0.0), Vector::new(1.0, 0.5, 3.0), TOL).expect("prism");
    assert_close(volume_of(&sheared), 4.0 * 3.0, 1e-9);
}

#[test]
fn extrude_refuses_the_mesh_tiers_bad_profiles() {
    assert!(matches!(
        solid::extrude(&square(2.0, 0.0), Vector::new(1.0, 0.0, 0.0), TOL),
        Err(GeomError::BadParameter {
            name: "direction",
            ..
        })
    ));
    let open = Curve::Line(Line {
        a: Point::origin(),
        b: Point::new(1.0, 0.0, 0.0),
    });
    assert!(matches!(
        solid::extrude(&open, Vector::new(0.0, 0.0, 1.0), TOL),
        Err(GeomError::OpenCurve { variant: "Line" })
    ));
    // Self-intersecting with non-zero area (a symmetric bow tie fails the
    // Newell frame first, as zero-area).
    let bowtie = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(4.0, 0.0, 0.0),
            Point::new(1.0, 2.0, 0.0),
            Point::new(3.0, 2.0, 0.0),
        ],
        closed: true,
    });
    assert!(matches!(
        solid::extrude(&bowtie, Vector::new(0.0, 0.0, 1.0), TOL),
        Err(GeomError::NotSimple { .. })
    ));
    let bent = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 0.0),
            Point::new(2.0, 2.0, 1.0),
            Point::new(0.0, 2.0, 0.0),
        ],
        closed: true,
    });
    assert!(matches!(
        solid::extrude(&bent, Vector::new(0.0, 0.0, 1.0), TOL),
        Err(GeomError::NotPlanar { .. })
    ));
}

#[test]
fn extrude_to_point_makes_pyramids_and_cones() {
    let pyramid = solid::extrude_to_point(&square(2.0, 0.0), Point::new(1.0, 1.0, 3.0), TOL)
        .expect("pyramid");
    assert_close(volume_of(&pyramid), 4.0 * 3.0 / 3.0, 1e-9);
    let (_, _, faces) = solid::edges_and_vertices(&pyramid, fine()).expect("topology");
    assert_eq!(faces, 5);
    let cone = solid::extrude_to_point(
        &circle(1.0, Plane::world_xy()),
        Point::new(0.0, 0.0, 3.0),
        TOL,
    )
    .expect("cone");
    assert_close(volume_of(&cone), PI / 3.0 * 3.0, 1e-6);
    assert!(matches!(
        solid::extrude_to_point(&square(2.0, 0.0), Point::new(5.0, 5.0, 0.0), TOL),
        Err(GeomError::BadParameter { name: "apex", .. })
    ));
}

#[test]
fn loft_between_squares_is_the_exact_frustum() {
    let frustum =
        solid::loft(&[square(2.0, 0.0), square_at(1.0, 0.5, 2.0)], true, TOL).expect("loft");
    // h/3 · (A₁ + A₂ + √(A₁A₂)) with A₁ = 4, A₂ = 1, h = 2.
    assert_close(volume_of(&frustum), 2.0 / 3.0 * (4.0 + 1.0 + 2.0), 1e-9);
    // Three sections, ruled: two stacked frusta.
    let tower = solid::loft(
        &[
            square(2.0, 0.0),
            square_at(1.0, 0.5, 2.0),
            square_at(2.0, 0.0, 4.0),
        ],
        true,
        TOL,
    )
    .expect("loft");
    assert_close(volume_of(&tower), 2.0 * 2.0 / 3.0 * 7.0, 1e-9);
    // Smooth through the same three: a B-spline surface through the
    // sections — the same ends, a different (smaller: the spline hugs the
    // waist) volume than the ruled chords.
    let smooth = solid::loft(
        &[
            square(2.0, 0.0),
            square_at(1.0, 0.5, 2.0),
            square_at(2.0, 0.0, 4.0),
        ],
        false,
        TOL,
    )
    .expect("smooth loft");
    let (min, max) = solid::bounds(&smooth).expect("bounds");
    assert!(tol::coincident(min, Point::origin(), 1e-6), "{min:?}");
    assert!(
        tol::coincident(max, Point::new(2.0, 2.0, 4.0), 1e-6),
        "{max:?}"
    );
    let (smooth_volume, ruled_volume) = (volume_of(&smooth), volume_of(&tower));
    assert!(smooth_volume > 0.0);
    assert!(
        (smooth_volume - ruled_volume).abs() > 1e-3,
        "smooth {smooth_volume} vs ruled {ruled_volume}: the port makes a difference"
    );
    // Circle to circle: a cone frustum, exactly.
    let cone = solid::loft(
        &[circle(2.0, Plane::world_xy()), circle(1.0, xy_at(3.0))],
        true,
        TOL,
    )
    .expect("loft");
    assert_close(volume_of(&cone), PI * 3.0 / 3.0 * (4.0 + 1.0 + 2.0), 1e-6);
    assert!(matches!(
        solid::loft(&[square(2.0, 0.0)], true, TOL),
        Err(GeomError::BadParameter {
            name: "profiles",
            ..
        })
    ));
    let error =
        solid::loft(&[square(2.0, 0.0), square(2.0, 0.0)], true, TOL).expect_err("coincident");
    assert!(matches!(error, GeomError::Kernel { .. }), "{error}");
}

fn square_at(side: f64, offset: f64, z: f64) -> Curve {
    Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(offset, offset, z),
            Point::new(offset + side, offset, z),
            Point::new(offset + side, offset + side, z),
            Point::new(offset, offset + side, z),
        ],
        closed: true,
    })
}

#[test]
#[allow(clippy::too_many_lines)] // one story: every angle form of one profile
fn revolve_makes_rings_and_partial_turns() {
    // A 1 × 1 square at x ∈ [2, 3] in the xz plane, about the z axis: a
    // square-section ring, V = 2π · R̄ · A = 2π · 2.5 · 1.
    let profile = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(2.0, 0.0, 0.0),
            Point::new(3.0, 0.0, 0.0),
            Point::new(3.0, 0.0, 1.0),
            Point::new(2.0, 0.0, 1.0),
        ],
        closed: true,
    });
    let z_axis = Curve::Line(Line {
        a: Point::origin(),
        b: Point::new(0.0, 0.0, 1.0),
    });
    let ring =
        solid::revolve(&profile, &z_axis, Domain::new(0.0, TAU), TOL, TOL_ANGLE).expect("ring");
    assert_close(volume_of(&ring), TAU * 2.5, 1e-9);
    // A quarter turn has a quarter of the volume, and starting at π/2 moves
    // it into the second quadrant (exact rigid transform).
    let quarter = solid::revolve(
        &profile,
        &z_axis,
        Domain::new(0.0, PI / 2.0),
        TOL,
        TOL_ANGLE,
    )
    .expect("quarter");
    assert_close(volume_of(&quarter), TAU * 2.5 / 4.0, 1e-9);
    let (min, max) = solid::bounds(&quarter).expect("bounds");
    assert!(min.0.x >= -1e-7 && min.0.y >= -1e-7, "{min:?}");
    assert!(
        tol::coincident(max, Point::new(3.0, 3.0, 1.0), 1e-7),
        "{max:?}"
    );
    let second = solid::revolve(&profile, &z_axis, Domain::new(PI / 2.0, PI), TOL, TOL_ANGLE)
        .expect("second quadrant");
    let (min, max) = solid::bounds(&second).expect("bounds");
    assert!(max.0.x <= 1e-7 && min.0.y >= -1e-7, "{min:?} {max:?}");
    assert_close(volume_of(&second), TAU * 2.5 / 4.0, 1e-9);
    // A negative sweep turns the other way.
    let backwards = solid::revolve(
        &profile,
        &z_axis,
        Domain::new(0.0, -PI / 2.0),
        TOL,
        TOL_ANGLE,
    )
    .expect("back");
    let (min, max) = solid::bounds(&backwards).expect("bounds");
    assert!(max.0.y <= 1e-7 && min.0.x >= -1e-7, "{min:?} {max:?}");
    // A profile touching the axis is fine (a disc); crossing it is refused.
    let disc_profile = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 1.0),
            Point::new(0.0, 0.0, 1.0),
        ],
        closed: true,
    });
    let disc = solid::revolve(
        &disc_profile,
        &z_axis,
        Domain::new(0.0, TAU),
        TOL,
        TOL_ANGLE,
    )
    .expect("disc");
    assert_close(volume_of(&disc), PI * 4.0, 1e-9);
    let crossing = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(-1.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 0.0),
            Point::new(2.0, 0.0, 1.0),
            Point::new(-1.0, 0.0, 1.0),
        ],
        closed: true,
    });
    assert!(matches!(
        solid::revolve(&crossing, &z_axis, Domain::new(0.0, TAU), TOL, TOL_ANGLE),
        Err(GeomError::BadParameter {
            name: "profile",
            ..
        })
    ));
    // The axis must lie in the profile plane; must be a line; the angle
    // must be a real, at most full, sweep.
    let off_plane = Curve::Line(Line {
        a: Point::new(0.0, 1.0, 0.0),
        b: Point::new(0.0, 1.0, 1.0),
    });
    assert!(matches!(
        solid::revolve(&profile, &off_plane, Domain::new(0.0, TAU), TOL, TOL_ANGLE),
        Err(GeomError::BadParameter { name: "axis", .. })
    ));
    assert!(matches!(
        solid::revolve(
            &profile,
            &circle(1.0, Plane::world_xy()),
            Domain::new(0.0, TAU),
            TOL,
            TOL_ANGLE
        ),
        Err(GeomError::BadParameter { name: "axis", .. })
    ));
    assert!(matches!(
        solid::revolve(&profile, &z_axis, Domain::new(1.0, 1.0), TOL, TOL_ANGLE),
        Err(GeomError::BadParameter { name: "angle", .. })
    ));
    assert!(matches!(
        solid::revolve(&profile, &z_axis, Domain::new(0.0, 7.0), TOL, TOL_ANGLE),
        Err(GeomError::BadParameter { name: "angle", .. })
    ));
}

#[test]
fn sweep_and_pipe_follow_their_rails() {
    // A unit square swept 5 along a straight rail is a 1 × 1 × 5 bar.
    let rail = Curve::Line(Line {
        a: Point::origin(),
        b: Point::new(0.0, 0.0, 5.0),
    });
    let bar = solid::sweep(&rail, &square(1.0, 0.0), TOL).expect("sweep");
    assert_close(volume_of(&bar), 5.0, 1e-6);
    // An L-shaped polyline rail with a mitred corner and the profile
    // centred on the rail: volume = area × length (the mitre takes from
    // one bar exactly what it gives the other).
    let elbow = Curve::Polyline(Polyline {
        vertices: vec![
            Point::origin(),
            Point::new(0.0, 0.0, 4.0),
            Point::new(3.0, 0.0, 4.0),
        ],
        closed: false,
    });
    let centred = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(-0.5, -0.5, 0.0),
            Point::new(0.5, -0.5, 0.0),
            Point::new(0.5, 0.5, 0.0),
            Point::new(-0.5, 0.5, 0.0),
        ],
        closed: true,
    });
    let swept = solid::sweep(&elbow, &centred, TOL).expect("sweep");
    assert_close(volume_of(&swept), 1.0 * 7.0, 1e-6);
    // The same profile sitting on the inside of the turn loses half the
    // corner cube from each bar: 4 − ½ + 3 − ½.
    let inside = solid::sweep(&elbow, &square(1.0, 0.0), TOL).expect("sweep");
    assert_close(volume_of(&inside), 6.0, 1e-6);
    // A pipe of radius r along a straight rail is a cylinder.
    let pipe = solid::pipe(&rail, 0.5, TOL).expect("pipe");
    assert_close(volume_of(&pipe), PI * 0.25 * 5.0, 1e-6);
    let (min, max) = solid::bounds(&pipe).expect("bounds");
    assert!(
        tol::coincident(min, Point::new(-0.5, -0.5, 0.0), 1e-6),
        "{min:?}"
    );
    assert!(
        tol::coincident(max, Point::new(0.5, 0.5, 5.0), 1e-6),
        "{max:?}"
    );
    // A pipe around a circle is a torus: V = 2π² R r².
    let ring = solid::pipe(&circle(3.0, Plane::world_xy()), 0.5, TOL).expect("torus");
    assert_close(volume_of(&ring), 2.0 * PI * PI * 3.0 * 0.25, 1e-3);
    assert!(matches!(
        solid::pipe(&rail, 0.0, TOL),
        Err(GeomError::BadParameter { name: "radius", .. })
    ));
}

// ---------------------------------------------------------------------------
// Booleans
// ---------------------------------------------------------------------------

fn cube(origin: Point, side: f64) -> Solid {
    solid::box_at(origin, Vector::new(side, side, side)).expect("cube")
}

#[test]
fn unions_merge_and_refuse_disjoint_bodies() {
    let a = cube(Point::origin(), 2.0);
    let b = cube(Point::new(1.0, 0.0, 0.0), 2.0);
    let c = cube(Point::new(2.0, 0.0, 0.0), 2.0);
    // Two overlapping cubes: 8 + 8 − 4.
    let ab = solid::union_all(&[a.clone(), b.clone()]).expect("union");
    assert_close(volume_of(&ab), 12.0, 1e-9);
    // Unified: the coplanar top faces merge back into one — a 3 × 2 × 2 box
    // has six faces, not the eight the raw fuse leaves.
    let (_, _, faces) = solid::edges_and_vertices(&ab, fine()).expect("topology");
    assert_eq!(faces, 6, "coplanar faces are merged");
    // Three in one pass (n-ary): 8 + 8 + 8 − 4 − 4 (b∩c) − 0 (a∩c is a face).
    let abc = solid::union_all(&[a.clone(), b, c]).expect("union");
    assert_close(volume_of(&abc), 16.0, 1e-9);
    // One solid passes through (re-serialized, equal bytes).
    assert_eq!(solid::union_all(std::slice::from_ref(&a)).expect("one"), a);
    // Disjoint: two bodies, refused.
    let far = cube(Point::new(10.0, 0.0, 0.0), 1.0);
    let error = solid::union_all(&[a, far]).expect_err("disjoint");
    assert!(matches!(error, GeomError::Kernel { .. }), "{error}");
    assert!(matches!(
        solid::union_all(&[]),
        Err(GeomError::BadParameter { name: "solids", .. })
    ));
}

#[test]
fn differences_take_every_cutter_in_one_pass() {
    let block = cube(Point::origin(), 4.0);
    let peg = |x: f64| {
        solid::cylinder(
            &Plane {
                origin: Point::new(x, 2.0, -1.0),
                ..Plane::world_xy()
            },
            0.5,
            6.0,
            TOL,
        )
        .expect("peg")
    };
    let drilled = solid::difference_all(&block, &[peg(1.0), peg(3.0)]).expect("cut");
    assert_close(volume_of(&drilled), 64.0 - 2.0 * PI * 0.25 * 4.0, 1e-9);
    // No cutters: the block, re-serialized.
    assert_eq!(solid::difference_all(&block, &[]).expect("none"), block);
    // The WP-B single cut and the n-ary cut of one cutter agree on volume
    // (topology may differ by unification; bytes are not compared).
    let one = solid::difference_all(&block, &[peg(1.0)]).expect("cut");
    let wp_b = solid::difference(&block, &peg(1.0)).expect("cut");
    assert_close(volume_of(&one), volume_of(&wp_b), 1e-9);
    // Splitting or emptying the solid is refused.
    let slab =
        solid::box_at(Point::new(-1.0, 1.5, -1.0), Vector::new(6.0, 1.0, 6.0)).expect("slab");
    assert!(matches!(
        solid::difference_all(&block, &[slab]),
        Err(GeomError::Kernel { .. })
    ));
    let everything = cube(Point::new(-1.0, -1.0, -1.0), 6.0);
    assert!(matches!(
        solid::difference_all(&block, &[everything]),
        Err(GeomError::Kernel { .. })
    ));
}

#[test]
fn intersections_keep_the_common_volume() {
    let a = cube(Point::origin(), 2.0);
    let b = cube(Point::new(1.0, 1.0, 1.0), 2.0);
    let common = solid::intersection(&a, &b).expect("common");
    assert_close(volume_of(&common), 1.0, 1e-9);
    let (min, max) = solid::bounds(&common).expect("bounds");
    assert!(tol::coincident(min, Point::new(1.0, 1.0, 1.0), 1e-9));
    assert!(tol::coincident(max, Point::new(2.0, 2.0, 2.0), 1e-9));
    let far = cube(Point::new(5.0, 5.0, 5.0), 1.0);
    assert!(matches!(
        solid::intersection(&a, &far),
        Err(GeomError::Kernel { .. })
    ));
}

// ---------------------------------------------------------------------------
// Transforms
// ---------------------------------------------------------------------------

#[test]
fn kernel_transforms_move_rotate_scale_and_mirror() {
    let block = solid::box_at(Point::origin(), Vector::new(1.0, 2.0, 3.0)).expect("box");
    let moved = solid::transform(
        &block,
        &Similarity::translation(Vector::new(10.0, 0.0, -1.0)),
    )
    .expect("move");
    let (min, max) = solid::bounds(&moved).expect("bounds");
    assert!(tol::coincident(min, Point::new(10.0, 0.0, -1.0), 1e-9));
    assert!(tol::coincident(max, Point::new(11.0, 2.0, 2.0), 1e-9));
    assert_close(volume_of(&moved), 6.0, 1e-9);
    // A quarter turn about z swaps the footprint.
    let frame = crate::frame::orthonormal(&Plane::world_xy(), TOL).expect("frame");
    let turned = solid::transform(&block, &Similarity::rotation(&frame, PI / 2.0)).expect("rotate");
    let (min, max) = solid::bounds(&turned).expect("bounds");
    assert!(
        tol::coincident(min, Point::new(-2.0, 0.0, 0.0), 1e-9),
        "{min:?}"
    );
    assert!(
        tol::coincident(max, Point::new(0.0, 1.0, 3.0), 1e-9),
        "{max:?}"
    );
    // Uniform scale about a point: volume × 8.
    let doubled =
        solid::transform(&block, &Similarity::scale_about(Point::origin(), 2.0)).expect("scale");
    assert_close(volume_of(&doubled), 48.0, 1e-9);
    // A point reflection (scale −1) keeps the volume positive: the kernel
    // re-orients the reversed solid.
    let mirrored =
        solid::transform(&block, &Similarity::scale_about(Point::origin(), -1.0)).expect("mirror");
    assert_close(volume_of(&mirrored), 6.0, 1e-9);
    let (min, max) = solid::bounds(&mirrored).expect("bounds");
    assert!(
        tol::coincident(min, Point::new(-1.0, -2.0, -3.0), 1e-9),
        "{min:?}"
    );
    assert!(tol::coincident(max, Point::origin(), 1e-9), "{max:?}");
    // The value-level transform is the one `Similarity::apply` takes.
    let via_apply = Similarity::translation(Vector::new(10.0, 0.0, -1.0))
        .apply(&cicada_core::geometry::Transformable::Solid(block));
    assert_eq!(
        via_apply,
        cicada_core::geometry::Transformable::Solid(moved)
    );
}

// ---------------------------------------------------------------------------
// Sections and topology
// ---------------------------------------------------------------------------

#[test]
fn sections_are_exact_where_the_kernel_is() {
    let cylinder = solid::cylinder(&Plane::world_xy(), 2.0, 4.0, TOL).expect("cylinder");
    let at_mid = solid::section(&cylinder, &xy_at(2.0), TOL, fine()).expect("section");
    assert_eq!(at_mid.len(), 1);
    let Curve::Circle(Circle { plane, radius }) = &at_mid[0] else {
        panic!("a cylinder's horizontal section is a circle: {at_mid:?}");
    };
    assert!(tol::close(*radius, 2.0, 1e-9));
    assert!(tol::coincident(
        plane.origin,
        Point::new(0.0, 0.0, 2.0),
        1e-9
    ));
    // A box's section is one closed polyline with four corners.
    let block = solid::box_at(Point::origin(), Vector::new(2.0, 3.0, 4.0)).expect("box");
    let cut = solid::section(&block, &xy_at(1.0), TOL, fine()).expect("section");
    assert_eq!(cut.len(), 1);
    let Curve::Polyline(Polyline { vertices, closed }) = &cut[0] else {
        panic!("a box's section is a polyline: {cut:?}");
    };
    assert!(*closed);
    assert_eq!(vertices.len(), 4);
    assert!(vertices.iter().all(|v| tol::close(v.0.z, 1.0, 1e-9)));
    // Two loops for a drilled block; none when the plane misses.
    let peg = solid::cylinder(
        &Plane {
            origin: Point::new(1.0, 1.5, -1.0),
            ..Plane::world_xy()
        },
        0.5,
        6.0,
        TOL,
    )
    .expect("peg");
    let drilled = solid::difference_all(&block, &[peg]).expect("cut");
    let loops = solid::section(&drilled, &xy_at(2.0), TOL, fine()).expect("section");
    assert_eq!(loops.len(), 2, "{loops:?}");
    assert!(loops.iter().any(|c| matches!(c, Curve::Circle(_))));
    assert!(
        solid::section(&block, &xy_at(9.0), TOL, fine())
            .expect("section")
            .is_empty()
    );
}

#[test]
fn deconstruct_reads_edges_vertices_and_faces() {
    let block = solid::box_at(Point::origin(), Vector::new(1.0, 2.0, 3.0)).expect("box");
    let (edges, vertices, faces) = solid::edges_and_vertices(&block, fine()).expect("topology");
    assert_eq!(faces, 6);
    assert_eq!(vertices.len(), 8);
    assert_eq!(edges.len(), 12);
    assert!(edges.iter().all(|e| matches!(e, Curve::Line(_))));
    let total: f64 = edges
        .iter()
        .map(|e| crate::curve::length(e, TOL).expect("length"))
        .sum();
    assert_close(total, 4.0 * (1.0 + 2.0 + 3.0), 1e-9);
    // A sphere: one face, one seam edge (a semicircle → polyline), two
    // poles.
    let sphere = solid::sphere(&Plane::world_xy(), 1.0, TOL).expect("sphere");
    let (edges, vertices, faces) = solid::edges_and_vertices(&sphere, fine()).expect("topology");
    assert_eq!(faces, 1);
    assert_eq!(vertices.len(), 2);
    assert_eq!(edges.len(), 1, "the degenerate pole edges are skipped");
}

// ---------------------------------------------------------------------------
// STEP
// ---------------------------------------------------------------------------

#[test]
fn step_round_trips_and_is_byte_deterministic() {
    let dir = std::env::temp_dir().join(format!("cicada-step-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("parts.step");
    let path_s = path.to_str().expect("utf-8 path");
    let block = solid::box_at(Point::origin(), Vector::new(1.0, 2.0, 3.0)).expect("box");
    let peg = solid::cylinder(&xy_at(5.0), 0.5, 2.0, TOL).expect("peg");
    let mm = ProjectConfig::default().unit().millimeters();
    solid::write_step(&[block.clone(), peg.clone()], path_s, mm, "parts").expect("write");
    let first = std::fs::read(&path).expect("bytes");
    let text = String::from_utf8_lossy(&first);
    assert!(text.starts_with("ISO-10303-21;"), "a STEP file");
    assert!(text.contains(STEP_TIMESTAMP), "the fixed timestamp");
    assert!(text.contains("'parts'"), "the name");
    assert!(
        text.contains("MILLI"),
        "millimetres declared: {}",
        &text[..600.min(text.len())]
    );
    // Written again: identical bytes (no wall-clock, no counters).
    solid::write_step(&[block.clone(), peg.clone()], path_s, mm, "parts").expect("write");
    assert_eq!(std::fs::read(&path).expect("bytes"), first);
    // Read back: two solids, the same volumes and bounds.
    let back = solid::read_step(path_s, mm).expect("read");
    assert_eq!(back.len(), 2);
    let volumes: Vec<f64> = back.iter().map(volume_of).collect();
    assert_close(volumes[0], 6.0, 1e-9);
    assert_close(volumes[1], PI * 0.25 * 2.0, 1e-9);
    let (min, max) = solid::bounds(&back[1]).expect("bounds");
    assert!(
        tol::coincident(min, Point::new(-0.5, -0.5, 5.0), 1e-6),
        "{min:?}"
    );
    assert!(
        tol::coincident(max, Point::new(0.5, 0.5, 7.0), 1e-6),
        "{max:?}"
    );
    // Units: an inch document writes INCH and reads back scaled in a
    // millimetre document.
    let inch_path = dir.join("inch.step");
    let inch_s = inch_path.to_str().expect("utf-8");
    solid::write_step(std::slice::from_ref(&block), inch_s, 25.4, "inch").expect("write");
    let inch_text = std::fs::read_to_string(&inch_path).expect("text");
    assert!(
        inch_text.contains("INCH"),
        "{}",
        &inch_text[..800.min(inch_text.len())]
    );
    let in_mm = solid::read_step(inch_s, 1.0).expect("read");
    assert_eq!(in_mm.len(), 1);
    assert_close(volume_of(&in_mm[0]), 6.0 * 25.4 * 25.4 * 25.4, 1e-6);
    let in_inches = solid::read_step(inch_s, 25.4).expect("read");
    assert_close(volume_of(&in_inches[0]), 6.0, 1e-6);
    // Failures are errors: a missing file, an unwritable path, no solids.
    assert!(matches!(
        solid::read_step(dir.join("missing.step").to_str().expect("utf-8"), mm),
        Err(GeomError::Kernel { .. })
    ));
    assert!(matches!(
        solid::write_step(
            std::slice::from_ref(&block),
            dir.join("no-such-dir")
                .join("x.step")
                .to_str()
                .expect("utf-8"),
            mm,
            "x"
        ),
        Err(GeomError::Kernel { .. })
    ));
    assert!(matches!(
        solid::write_step(&[], path_s, mm, "x"),
        Err(GeomError::BadParameter { name: "solids", .. })
    ));
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn step_calls_are_safe_across_threads() {
    // The STEP lock serializes the translators' global state: eight
    // threads writing and reading distinct files at once all succeed and
    // read back what they wrote.
    use rayon::prelude::*;
    let dir = std::env::temp_dir().join(format!("cicada-step-par-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .expect("pool");
    let failures: Vec<String> = pool.install(|| {
        (0..16u32)
            .into_par_iter()
            .filter_map(|i| {
                let side = f64::from(i + 1);
                let block = cube(Point::origin(), side);
                let path = dir.join(format!("{i}.step"));
                let path_s = path.to_str().expect("utf-8");
                if let Err(e) = solid::write_step(std::slice::from_ref(&block), path_s, 1.0, "t") {
                    return Some(format!("{i}: write {e}"));
                }
                match solid::read_step(path_s, 1.0) {
                    Ok(back) if back.len() == 1 => {
                        let v = solid::volume(&back[0]).expect("volume").volume;
                        ((v - side * side * side).abs() > 1e-6).then(|| format!("{i}: volume {v}"))
                    }
                    Ok(back) => Some(format!("{i}: {} solids", back.len())),
                    Err(e) => Some(format!("{i}: read {e}")),
                }
            })
            .collect()
    });
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert!(failures.is_empty(), "{failures:?}");
}

// ---------------------------------------------------------------------------
// Heap independence of the node set's golden corpus (WP-B's review asked
// for this shape on loft / revolve / multi-body booleans BEFORE their
// goldens are blessed; the stdlib's goldens are these solids)
// ---------------------------------------------------------------------------

/// The transcendental-free solids the stdlib pins goldens on (plus the
/// revolve and sweep, which have no golden but the same question): each a
/// pure function of exact inputs if the kernel is one.
fn golden_corpus() -> Vec<(&'static str, Solid)> {
    let pentagon = |z: f64, pts: &[(f64, f64)]| {
        Curve::Polyline(Polyline {
            vertices: pts.iter().map(|&(x, y)| Point::new(x, y, z)).collect(),
            closed: true,
        })
    };
    let cell = pentagon(
        0.0,
        &[(0.0, 0.0), (4.0, 0.0), (5.0, 3.0), (2.0, 5.0), (-1.0, 3.0)],
    );
    let cap = pentagon(
        12.0,
        &[(2.0, 1.0), (2.75, 1.5), (3.5, 2.0), (2.0, 3.5), (0.5, 2.0)],
    );
    let ring_profile = Curve::Polyline(Polyline {
        vertices: vec![
            Point::new(2.0, 0.0, 0.0),
            Point::new(3.0, 0.0, 0.0),
            Point::new(3.0, 0.0, 1.0),
            Point::new(2.0, 0.0, 1.0),
        ],
        closed: true,
    });
    let z_axis = Curve::Line(Line {
        a: Point::origin(),
        b: Point::new(0.0, 0.0, 1.0),
    });
    let elbow = Curve::Polyline(Polyline {
        vertices: vec![
            Point::origin(),
            Point::new(0.0, 0.0, 4.0),
            Point::new(3.0, 0.0, 4.0),
        ],
        closed: false,
    });
    vec![
        ("loft", solid::loft(&[cell, cap], true, TOL).expect("loft")),
        (
            "union of three",
            solid::union_all(&[
                cube(Point::origin(), 2.0),
                cube(Point::new(1.0, 0.0, 0.0), 2.0),
                cube(Point::new(2.0, 0.0, 0.0), 2.0),
            ])
            .expect("union"),
        ),
        (
            "n-ary cut",
            solid::difference_all(
                &solid::box_at(Point::origin(), Vector::new(4.0, 3.0, 2.0)).expect("box"),
                &[
                    cube(Point::new(3.0, 2.0, 1.0), 2.0),
                    cube(Point::new(-1.0, -1.0, -1.0), 2.0),
                ],
            )
            .expect("cut"),
        ),
        (
            "intersection",
            solid::intersection(
                &solid::box_at(Point::origin(), Vector::new(4.0, 3.0, 2.0)).expect("box"),
                &solid::box_at(Point::new(1.0, 1.0, 1.0), Vector::new(4.0, 3.0, 2.0)).expect("box"),
            )
            .expect("common"),
        ),
        (
            "extrude to point",
            solid::extrude_to_point(&square(2.0, 0.0), Point::new(0.5, 0.25, 3.0), TOL)
                .expect("pyramid"),
        ),
        (
            "revolve",
            solid::revolve(
                &ring_profile,
                &z_axis,
                Domain::new(0.0, TAU),
                TOL,
                TOL_ANGLE,
            )
            .expect("ring"),
        ),
        (
            "sweep",
            solid::sweep(&elbow, &square(1.0, 0.0), TOL).expect("sweep"),
        ),
    ]
}

#[test]
fn node_set_bytes_do_not_depend_on_heap_state_or_thread() {
    // The shape of `canonical_bytes_do_not_depend_on_heap_state_or_thread`
    // (occt/tests.rs) over the node set's corpus: computed cold, after
    // deterministic allocator churn under several seeds, and on 8 threads
    // at once under per-worker churn — every result byte-identical.
    use super::tests::churn_heap;
    use rayon::prelude::*;
    const THREADS: usize = 8;
    const REPEATS: usize = 16;

    let golden = golden_corpus();
    for seed in 1..=4u64 {
        let _ = churn_heap(seed, 2_000);
        let again = golden_corpus();
        for ((name, want), (_, got)) in golden.iter().zip(&again) {
            assert_eq!(got, want, "seed {seed}: {name} followed the heap");
        }
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .expect("pool");
    let failures: Vec<String> = pool.install(|| {
        (0..REPEATS)
            .into_par_iter()
            .filter_map(|repeat| {
                let seed = 100 + u64::try_from(repeat).expect("small");
                let _ = churn_heap(seed, 1_000 + repeat * 37);
                let again = golden_corpus();
                golden
                    .iter()
                    .zip(&again)
                    .find(|((_, want), (_, got))| got != want)
                    .map(|((name, _), _)| format!("repeat {repeat}: {name} drifted"))
            })
            .collect()
    });
    assert!(failures.is_empty(), "{failures:?}");
}
