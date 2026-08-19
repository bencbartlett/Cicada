//! Mesh construction: extrude, box, sphere — the spike's mesh-backed
//! primitives (doc 15's honest shim: v0.1 node names, mesh-tier bodies).
//! Every builder produces a structurally watertight, outward-oriented mesh
//! or refuses loudly; watertightness is debug-asserted at the seam.

use cicada_core::geometry::{Curve, Mesh};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use glam::DVec2;

use crate::frame::{orthonormal, polygon_frame};
use crate::triangulate::{ear_clip, signed_area_doubled};
use crate::{GeomError, curve as curve_ops, tol};

fn debug_assert_watertight(mesh: &Mesh, builder: &str) {
    debug_assert!(
        mesh.is_watertight(),
        "{builder} produced a non-watertight mesh — builder bug"
    );
}

/// Extrude a closed profile along a direction into a watertight prism.
/// `segments` tessellates curved profiles (circles); vertex-chain profiles
/// use their own corners. The profile must be planar within `tolerance`,
/// and the direction must actually leave the profile plane.
///
/// # Errors
///
/// [`GeomError`]: open profile, degenerate profile/frame, non-planar
/// profile, in-plane direction, or a non-simple polygon.
pub fn extrude(
    profile: &Curve,
    direction: Vector,
    segments: i64,
    tolerance: f64,
) -> Result<Mesh, GeomError> {
    let loop_points = curve_ops::tessellate_closed(profile, segments, tolerance)?;
    let frame = polygon_frame(&loop_points, tolerance)?;
    let mut flat = Vec::with_capacity(loop_points.len());
    for (vertex, point) in loop_points.iter().enumerate() {
        let local = frame.coordinates(*point);
        if !tol::near_zero(local.z, tolerance) {
            return Err(GeomError::NotPlanar {
                vertex,
                distance: local.z,
            });
        }
        flat.push(DVec2::new(local.x, local.y));
    }
    let height = direction.0.dot(frame.z);
    if tol::near_zero(height, tolerance) {
        return Err(GeomError::BadParameter {
            name: "direction",
            value: format!("{:?}", direction.0),
            requirement: "must leave the profile plane (not be parallel to it)",
        });
    }

    // Counter-clockwise boundary order (walls walk it; caps come from
    // ear_clip already CCW-normalized). Raw sign test sanctioned: the
    // ambiguous near-zero-area band is excluded by ear_clip's own
    // zero-area refusal on the same polygon (tol discipline, doc 14).
    let n = flat.len();
    let ccw: Vec<u32> = if signed_area_doubled(&flat) < 0.0 {
        #[allow(clippy::cast_possible_truncation)]
        (0..n as u32).rev().collect()
    } else {
        #[allow(clippy::cast_possible_truncation)]
        (0..n as u32).collect()
    };
    let cap = ear_clip(&flat, tolerance)?;

    // Vertices: bottom loop then top loop.
    let mut positions = Vec::with_capacity(n * 2 * 3);
    for point in &loop_points {
        positions.extend_from_slice(&[point.0.x, point.0.y, point.0.z]);
    }
    for point in &loop_points {
        let top = point.0 + direction.0;
        positions.extend_from_slice(&[top.x, top.y, top.z]);
    }
    #[allow(clippy::cast_possible_truncation)]
    let top_offset = n as u32;

    // `up`: direction is on the frame's +z side. CCW cap triangles have +z
    // normals, so: up → top cap as-is, bottom reversed; down → mirrored.
    // Raw sign test sanctioned: |height| <= tolerance was refused above,
    // so the ambiguous band is already excluded (tol discipline, doc 14).
    let up = height > 0.0;
    let mut indices = Vec::with_capacity((cap.len() * 2 + n * 2) * 3);
    for triangle in &cap {
        let [a, b, c] = *triangle;
        if up {
            indices.extend_from_slice(&[a, c, b]); // bottom, −z out
            indices.extend_from_slice(&[a + top_offset, b + top_offset, c + top_offset]);
        } else {
            indices.extend_from_slice(&[a, b, c]); // bottom faces +z = out
            indices.extend_from_slice(&[a + top_offset, c + top_offset, b + top_offset]);
        }
    }
    for (k, &i) in ccw.iter().enumerate() {
        let j = ccw[(k + 1) % n];
        let (bi, bj, ti, tj) = (i, j, i + top_offset, j + top_offset);
        if up {
            indices.extend_from_slice(&[bi, bj, tj]);
            indices.extend_from_slice(&[bi, tj, ti]);
        } else {
            indices.extend_from_slice(&[bi, tj, bj]);
            indices.extend_from_slice(&[bi, ti, tj]);
        }
    }
    let mesh = Mesh::new(positions, indices)?;
    debug_assert_watertight(&mesh, "extrude");
    Ok(mesh)
}

/// An axis-aligned box in a plane's frame, spanning `x × y × z` domains
/// (decreasing domains are normalized).
///
/// # Errors
///
/// [`GeomError`]: degenerate frame, or an extent empty at `tolerance`.
pub fn box_mesh(
    plane: &Plane,
    x: Domain,
    y: Domain,
    z: Domain,
    tolerance: f64,
) -> Result<Mesh, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    let span = |d: &Domain, name: &'static str| -> Result<(f64, f64), GeomError> {
        if tol::close(d.start, d.end, tolerance) {
            return Err(GeomError::BadParameter {
                name,
                value: format!("{}..{}", d.start, d.end),
                requirement: "extent must exceed tolerance",
            });
        }
        Ok((d.start.min(d.end), d.start.max(d.end)))
    };
    let (x0, x1) = span(&x, "x")?;
    let (y0, y1) = span(&y, "y")?;
    let (z0, z1) = span(&z, "z")?;

    let corners = [
        (x0, y0, z0),
        (x1, y0, z0),
        (x1, y1, z0),
        (x0, y1, z0),
        (x0, y0, z1),
        (x1, y0, z1),
        (x1, y1, z1),
        (x0, y1, z1),
    ];
    let mut positions = Vec::with_capacity(24);
    for &(u, v, w) in &corners {
        let p = frame.point_at_3(u, v, w);
        positions.extend_from_slice(&[p.0.x, p.0.y, p.0.z]);
    }
    // Outward faces for a right-handed frame; fixed order = fixed hash.
    let indices: Vec<u32> = vec![
        0, 2, 1, 0, 3, 2, // bottom (−z)
        4, 5, 6, 4, 6, 7, // top (+z)
        0, 1, 5, 0, 5, 4, // front (−y)
        1, 2, 6, 1, 6, 5, // right (+x)
        2, 3, 7, 2, 7, 6, // back (+y)
        3, 0, 4, 3, 4, 7, // left (−x)
    ];
    let mesh = Mesh::new(positions, indices)?;
    debug_assert_watertight(&mesh, "box");
    Ok(mesh)
}

/// A UV sphere: `segments` longitudes (≥ 3), `segments/2` latitude bands
/// (minimum 2), poles as fans. Centered at the plane's origin with the
/// plane's z as the polar axis.
///
/// # Errors
///
/// [`GeomError`]: degenerate frame, radius not above tolerance, or
/// `segments < 3`.
pub fn sphere_mesh(
    plane: &Plane,
    radius: f64,
    segments: i64,
    tolerance: f64,
) -> Result<Mesh, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    if radius <= tolerance {
        return Err(GeomError::BadParameter {
            name: "radius",
            value: radius.to_string(),
            requirement: "must exceed tolerance",
        });
    }
    if segments < 3 {
        return Err(GeomError::BadParameter {
            name: "segments",
            value: segments.to_string(),
            requirement: "must be >= 3",
        });
    }
    let rings = (segments / 2).max(2);

    let point_on = |ring: i64, seg: i64| -> Point {
        #[allow(clippy::cast_precision_loss)]
        let theta = std::f64::consts::PI * ring as f64 / rings as f64;
        #[allow(clippy::cast_precision_loss)]
        let phi = std::f64::consts::TAU * seg as f64 / segments as f64;
        let (st, ct) = theta.sin_cos();
        let (sp, cp) = phi.sin_cos();
        frame.point_at_3(radius * st * cp, radius * st * sp, radius * ct)
    };

    let mut positions = Vec::new();
    let push = |positions: &mut Vec<f64>, p: Point| {
        positions.extend_from_slice(&[p.0.x, p.0.y, p.0.z]);
    };
    push(&mut positions, frame.point_at_3(0.0, 0.0, radius)); // north, index 0
    for ring in 1..rings {
        for seg in 0..segments {
            push(&mut positions, point_on(ring, seg));
        }
    }
    push(&mut positions, frame.point_at_3(0.0, 0.0, -radius)); // south, last
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (segments_u, south) = (segments as u32, (1 + (rings - 1) * segments) as u32);
    let ring_vertex = |ring: u32, seg: u32| 1 + (ring - 1) * segments_u + (seg % segments_u);

    let mut indices = Vec::new();
    for seg in 0..segments_u {
        indices.extend_from_slice(&[0, ring_vertex(1, seg), ring_vertex(1, seg + 1)]);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let band_count = (rings - 1) as u32;
    for ring in 1..band_count {
        for seg in 0..segments_u {
            let (a, b) = (ring_vertex(ring, seg), ring_vertex(ring, seg + 1));
            let (d, c) = (ring_vertex(ring + 1, seg), ring_vertex(ring + 1, seg + 1));
            indices.extend_from_slice(&[a, d, c]);
            indices.extend_from_slice(&[a, c, b]);
        }
    }
    for seg in 0..segments_u {
        indices.extend_from_slice(&[
            south,
            ring_vertex(band_count, seg + 1),
            ring_vertex(band_count, seg),
        ]);
    }
    let mesh = Mesh::new(positions, indices)?;
    debug_assert_watertight(&mesh, "sphere");
    Ok(mesh)
}

/// Signed volume of a watertight mesh (divergence theorem over triangles).
/// Positive for outward orientation — the builders' unit tests and the
/// carve benchmark assert with it.
#[must_use]
pub fn signed_volume(mesh: &Mesh) -> f64 {
    let positions = mesh.positions();
    let mut six_volumes = 0.0;
    for tri in mesh.indices().chunks_exact(3) {
        let vertex = |index: u32| {
            let at = index as usize * 3;
            glam::DVec3::new(positions[at], positions[at + 1], positions[at + 2])
        };
        let (va, vb, vc) = (vertex(tri[0]), vertex(tri[1]), vertex(tri[2]));
        six_volumes += va.dot(vb.cross(vc));
    }
    six_volumes / 6.0
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Polyline, Rectangle};

    use super::*;

    const TOL: f64 = 1e-6;

    fn world_xy() -> Plane {
        Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        }
    }

    #[test]
    fn box_is_watertight_with_exact_volume() {
        let mesh = box_mesh(
            &world_xy(),
            Domain::new(0.0, 2.0),
            Domain::new(-1.0, 1.0),
            Domain::new(0.0, 3.0),
            TOL,
        )
        .expect("builds");
        assert!(mesh.is_watertight());
        assert_eq!(mesh.triangle_count(), 12);
        assert!((signed_volume(&mesh) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn decreasing_domains_normalize_to_the_same_box() {
        let a = box_mesh(
            &world_xy(),
            Domain::new(0.0, 2.0),
            Domain::new(-1.0, 1.0),
            Domain::new(0.0, 3.0),
            TOL,
        )
        .expect("builds");
        let b = box_mesh(
            &world_xy(),
            Domain::new(2.0, 0.0),
            Domain::new(1.0, -1.0),
            Domain::new(3.0, 0.0),
            TOL,
        )
        .expect("builds");
        assert_eq!(a, b);
    }

    #[test]
    fn extrude_rectangle_up_and_down_both_orient_outward() {
        let profile = Curve::Rectangle(Rectangle {
            plane: world_xy(),
            x: Domain::new(0.0, 2.0),
            y: Domain::new(0.0, 1.0),
        });
        for dz in [3.0, -3.0] {
            let mesh = extrude(&profile, Vector::new(0.0, 0.0, dz), 64, TOL).expect("extrudes");
            assert!(mesh.is_watertight());
            assert!(
                (signed_volume(&mesh) - 6.0).abs() < 1e-9,
                "outward volume positive for dz={dz}"
            );
        }
    }

    #[test]
    fn extrude_circle_volume_approximates_cylinder() {
        let profile = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 1.0,
        });
        let mesh = extrude(&profile, Vector::new(0.0, 0.0, 2.0), 256, TOL).expect("extrudes");
        assert!(mesh.is_watertight());
        let expected = std::f64::consts::PI * 2.0; // πr²h
        let got = signed_volume(&mesh);
        assert!(
            (got - expected).abs() / expected < 1e-3,
            "256-gon prism ≈ cylinder: got {got}, want ≈ {expected}"
        );
    }

    #[test]
    fn extrude_concave_profile_is_watertight() {
        // The L shape again, closed, extruded obliquely.
        let profile = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(2.0, 0.0, 0.0),
                Point::new(2.0, 1.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
                Point::new(1.0, 2.0, 0.0),
                Point::new(0.0, 2.0, 0.0),
            ],
            closed: true,
        });
        let mesh = extrude(&profile, Vector::new(0.25, -0.5, 1.5), 8, TOL).expect("extrudes");
        assert!(mesh.is_watertight());
        // Oblique prism: volume = base area × height component ⊥ base.
        assert!((signed_volume(&mesh) - 3.0 * 1.5).abs() < 1e-9);
    }

    #[test]
    fn extrude_refusals_are_loud() {
        let profile = Curve::Rectangle(Rectangle {
            plane: world_xy(),
            x: Domain::new(0.0, 1.0),
            y: Domain::new(0.0, 1.0),
        });
        // In-plane direction.
        assert!(matches!(
            extrude(&profile, Vector::new(1.0, 0.0, 0.0), 8, TOL),
            Err(GeomError::BadParameter {
                name: "direction",
                ..
            })
        ));
        // Non-planar profile.
        let skew = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.5),
                Point::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        });
        assert!(matches!(
            extrude(&skew, Vector::new(0.0, 0.0, 1.0), 8, TOL),
            Err(GeomError::NotPlanar { .. })
        ));
    }

    #[test]
    fn sphere_is_watertight_and_converges_to_ball_volume() {
        let mesh = sphere_mesh(&world_xy(), 1.0, 64, TOL).expect("builds");
        assert!(mesh.is_watertight());
        let expected = 4.0 / 3.0 * std::f64::consts::PI;
        let got = signed_volume(&mesh);
        assert!(
            (got - expected).abs() / expected < 5e-3,
            "64-segment sphere ≈ ball: got {got}, want ≈ {expected}"
        );
    }

    #[test]
    fn sphere_refusals_are_loud() {
        assert!(matches!(
            sphere_mesh(&world_xy(), 0.0, 16, TOL),
            Err(GeomError::BadParameter { name: "radius", .. })
        ));
        assert!(matches!(
            sphere_mesh(&world_xy(), 1.0, 2, TOL),
            Err(GeomError::BadParameter {
                name: "segments",
                ..
            })
        ));
    }

    proptest::proptest! {
        // Any box: watertight, 12 triangles, volume = product of extents.
        #[test]
        fn property_box_volume(
            x0 in -10.0f64..10.0, dx in 0.01f64..10.0,
            y0 in -10.0f64..10.0, dy in 0.01f64..10.0,
            z0 in -10.0f64..10.0, dz in 0.01f64..10.0,
        ) {
            let mesh = box_mesh(
                &world_xy(),
                Domain::new(x0, x0 + dx),
                Domain::new(y0, y0 + dy),
                Domain::new(z0, z0 + dz),
                TOL,
            ).expect("builds");
            proptest::prop_assert!(mesh.is_watertight());
            let want = dx * dy * dz;
            proptest::prop_assert!((signed_volume(&mesh) - want).abs() <= 1e-9 * want.max(1.0));
        }

        // Any regular-polygon prism: watertight with exact prism volume.
        #[test]
        fn property_extrude_prism_volume(
            sides in 3i64..24,
            radius in 0.1f64..10.0,
            height in 0.1f64..10.0,
        ) {
            let profile = Curve::Circle(Circle { plane: world_xy(), radius });
            let mesh = extrude(&profile, Vector::new(0.0, 0.0, height), sides, TOL)
                .expect("extrudes");
            proptest::prop_assert!(mesh.is_watertight());
            // Regular n-gon area: n/2 · r² · sin(2π/n).
            #[allow(clippy::cast_precision_loss)]
            let base = sides as f64 / 2.0 * radius * radius
                * (std::f64::consts::TAU / sides as f64).sin();
            let want = base * height;
            proptest::prop_assert!(
                (signed_volume(&mesh) - want).abs() <= 1e-9 * want.max(1.0)
            );
        }
    }
}
