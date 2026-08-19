//! Mesh construction: extrude, loft, box, sphere — the spike's mesh-backed
//! primitives (doc 15's honest shim: v0.1 node names, mesh-tier bodies).
//! Every builder produces a structurally watertight, outward-oriented mesh
//! or refuses loudly; watertightness is debug-asserted at the seam (and
//! re-verified for real by `loft`, whose contract promises it).

use cicada_core::geometry::{Curve, Mesh, Polyline};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use glam::{DVec2, DVec3};

use crate::frame::{orthonormal, polygon_frame};
use crate::triangulate::{ear_clip, signed_area_doubled};
use crate::{GeomError, curve as curve_ops, tol};

fn debug_assert_watertight(mesh: &Mesh, builder: &str) {
    debug_assert!(
        mesh.is_watertight(),
        "{builder} produced a non-watertight mesh — builder bug"
    );
}

/// Why a loft refused (stage 6). Every variant names the offending section
/// or carries the counts, honoring loud refusal (docs/08 rule 7); the `loft`
/// node surfaces the Display text as its red message.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LoftError {
    /// The sections do not have the same vertex count — they pair vertex
    /// `i` with vertex `i`, so there is no honest correspondence.
    #[error(
        "section vertex counts differ: start has {start}, end has {end} — sections pair \
         vertex i with vertex i (tessellate analytic sections to the same `segments`, or \
         rebuild the polylines)"
    )]
    VertexCountMismatch {
        /// Vertex count of the start section.
        start: usize,
        /// Vertex count of the end section.
        end: usize,
    },
    /// One section is unusable: open, degenerate at tolerance, non-planar,
    /// or not a simple polygon.
    #[error("{which} section: {source}")]
    Section {
        /// `start` or `end`.
        which: &'static str,
        /// What failed.
        source: GeomError,
    },
    /// The sections wind in opposite directions about the loft, so the walls
    /// would twist through themselves.
    #[error(
        "sections wind in opposite directions (their normals oppose) — the walls would twist \
         through themselves; reverse one section"
    )]
    OppositeWinding,
    /// The loft encloses no volume: the sections coincide or are coplanar.
    #[error(
        "zero volume: the sections coincide or are coplanar (effective height {height} is \
         within tolerance {tolerance})"
    )]
    ZeroVolume {
        /// Enclosed volume divided by the larger section area.
        height: f64,
        /// The tolerance it fell within.
        tolerance: f64,
    },
    /// The finished mesh failed the watertight check — a builder bug,
    /// refused rather than passed downstream as a `Watertight<Mesh>`.
    #[error("result is not watertight ({triangles} triangles) — loft builder bug")]
    NotWatertight {
        /// Triangle count of the refused mesh.
        triangles: usize,
    },
    /// A parameter or construction step refused (bad `segments`, mesh
    /// construction).
    #[error(transparent)]
    Geom(#[from] GeomError),
}

/// One cap of a loft: the triangulated section in loop order, its unit
/// normal (Newell), and its area.
struct Cap {
    triangles: Vec<[u32; 3]>,
    normal: DVec3,
    area: f64,
}

/// The vertex loop of one loft section (no duplicate closing vertex).
/// Closed polylines contribute their vertices as given — pairing is vertex
/// `i` ↔ vertex `i`, seam = vertex 0, so nothing is deduplicated or
/// resampled; analytic sections (circle, rectangle) are divided into
/// `segments` equal arc-length samples from their parameter origin (a
/// circle's +x axis; a rectangle's `(x.start, y.start)` corner).
fn loft_section(
    curve: &Curve,
    which: &'static str,
    segments: i64,
    tolerance: f64,
) -> Result<Vec<Point>, LoftError> {
    let section = |source: GeomError| LoftError::Section { which, source };
    if !curve.is_closed() {
        return Err(section(GeomError::OpenCurve {
            variant: curve.variant_name(),
        }));
    }
    match curve {
        Curve::Polyline(Polyline { vertices, .. }) => {
            if vertices.len() < 3 {
                return Err(section(GeomError::DegenerateCurve {
                    reason: format!("{} vertices (need 3)", vertices.len()),
                }));
            }
            for (i, &a) in vertices.iter().enumerate() {
                let j = (i + 1) % vertices.len();
                if tol::coincident(a, vertices[j], tolerance) {
                    return Err(section(GeomError::DegenerateCurve {
                        reason: format!(
                            "vertices {i} and {j} coincide at tolerance {tolerance} — a \
                             zero-length edge has no wall"
                        ),
                    }));
                }
            }
            Ok(vertices.clone())
        }
        Curve::Circle(_) | Curve::Rectangle(_) => {
            if segments < 3 {
                return Err(GeomError::BadParameter {
                    name: "segments",
                    value: segments.to_string(),
                    requirement: "must be >= 3",
                }
                .into());
            }
            Ok(curve_ops::divide(curve, segments, tolerance)
                .map_err(section)?
                .points)
        }
        Curve::Line(_) => unreachable!("lines are never closed"),
    }
}

/// Triangulate one planar section loop. The loop is projected into its own
/// polygon frame (z = Newell normal), so the 2D polygon is counter-clockwise
/// by construction and `ear_clip` returns triangles whose boundary edges run
/// in loop order.
fn loft_cap(loop_points: &[Point], which: &'static str, tolerance: f64) -> Result<Cap, LoftError> {
    let section = |source: GeomError| LoftError::Section { which, source };
    let frame = polygon_frame(loop_points, tolerance).map_err(section)?;
    let mut flat = Vec::with_capacity(loop_points.len());
    for (vertex, point) in loop_points.iter().enumerate() {
        let local = frame.coordinates(*point);
        if !tol::near_zero(local.z, tolerance) {
            return Err(section(GeomError::NotPlanar {
                vertex,
                distance: local.z,
            }));
        }
        flat.push(DVec2::new(local.x, local.y));
    }
    let area2 = signed_area_doubled(&flat);
    debug_assert!(
        area2 > 0.0,
        "projection into the Newell frame is counter-clockwise by construction"
    );
    let triangles = ear_clip(&flat, tolerance).map_err(section)?;
    Ok(Cap {
        triangles,
        normal: frame.z,
        area: area2 / 2.0,
    })
}

/// Loft a ruled solid between two closed sections, capped at both ends —
/// the wall's frusta (Voronoi cell → tip-cap polygon), cones and chamfers
/// (circle → circle). Sections pair vertex `i` with vertex `i` (seam =
/// vertex 0): polylines contribute their vertices as given; circles and
/// rectangles are divided into `segments` equal arc-length samples. Walls
/// are two triangles per quad (`(aᵢ, aᵢ₊₁, bᵢ₊₁)`, `(aᵢ, bᵢ₊₁, bᵢ)`), caps
/// are ear-clipped (non-convex sections welcome), and the whole mesh is
/// oriented outward by its signed volume, then re-verified watertight.
///
/// # Errors
///
/// [`LoftError`]: vertex counts differ (both counts), a section is open,
/// degenerate at tolerance, non-planar, or self-intersecting, the sections
/// wind in opposite directions, the loft has zero volume (coincident or
/// coplanar sections), `segments < 3` for an analytic section, or the
/// result is not watertight.
pub fn loft(start: &Curve, end: &Curve, segments: i64, tolerance: f64) -> Result<Mesh, LoftError> {
    let a = loft_section(start, "start", segments, tolerance)?;
    let b = loft_section(end, "end", segments, tolerance)?;
    if a.len() != b.len() {
        return Err(LoftError::VertexCountMismatch {
            start: a.len(),
            end: b.len(),
        });
    }
    let cap_a = loft_cap(&a, "start", tolerance)?;
    let cap_b = loft_cap(&b, "end", tolerance)?;
    // Raw sign test sanctioned: a dot below zero means the loops are
    // traversed in opposite senses about the loft (tol discipline, doc 14 —
    // the near-zero band is perpendicular sections, which are legal).
    if cap_a.normal.dot(cap_b.normal) < 0.0 {
        return Err(LoftError::OppositeWinding);
    }

    let n = a.len();
    #[allow(clippy::cast_possible_truncation)] // far below u32::MAX vertices
    let offset = n as u32;
    let mut positions = Vec::with_capacity(n * 2 * 3);
    for point in a.iter().chain(&b) {
        positions.extend_from_slice(&[point.0.x, point.0.y, point.0.z]);
    }
    // Combinatorially consistent orientation first (start cap against loop
    // order, end cap with it, walls from start to end), then the signed
    // volume decides whether the whole thing faces outward.
    let mut indices =
        Vec::with_capacity((cap_a.triangles.len() + cap_b.triangles.len() + 2 * n) * 3);
    for &[p, q, r] in &cap_a.triangles {
        indices.extend_from_slice(&[p, r, q]);
    }
    for &[p, q, r] in &cap_b.triangles {
        indices.extend_from_slice(&[p + offset, q + offset, r + offset]);
    }
    for i in 0..offset {
        let j = (i + 1) % offset;
        indices.extend_from_slice(&[i, j, j + offset]);
        indices.extend_from_slice(&[i, j + offset, i + offset]);
    }
    let mut mesh = Mesh::new(positions, indices).map_err(GeomError::from)?;
    let volume = signed_volume(&mesh);
    let height = volume.abs() / cap_a.area.max(cap_b.area);
    if height <= tolerance {
        return Err(LoftError::ZeroVolume { height, tolerance });
    }
    if volume < 0.0 {
        let flipped: Vec<u32> = mesh
            .indices()
            .chunks_exact(3)
            .flat_map(|tri| [tri[0], tri[2], tri[1]])
            .collect();
        mesh = Mesh::new(mesh.positions().to_vec(), flipped).map_err(GeomError::from)?;
    }
    if !mesh.is_watertight() {
        return Err(LoftError::NotWatertight {
            triangles: mesh.triangle_count(),
        });
    }
    Ok(mesh)
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

    // ------------------------------------------------------------ loft --

    fn polygon(points: &[(f64, f64, f64)]) -> Curve {
        Curve::Polyline(Polyline {
            vertices: points
                .iter()
                .map(|&(x, y, z)| Point::new(x, y, z))
                .collect(),
            closed: true,
        })
    }

    /// A polygon scaled about `center` by `scale` and lifted to `z` — the
    /// homothetic pair whose loft is an exact frustum.
    fn scaled_copy(points: &[(f64, f64, f64)], center: (f64, f64), scale: f64, z: f64) -> Curve {
        let lifted: Vec<(f64, f64, f64)> = points
            .iter()
            .map(|&(x, y, _)| {
                (
                    center.0 + (x - center.0) * scale,
                    center.1 + (y - center.1) * scale,
                    z,
                )
            })
            .collect();
        polygon(&lifted)
    }

    /// Frustum volume between homothetic sections: h/3 · (A₁ + A₂ + √(A₁A₂)).
    fn frustum_volume(area: f64, scale: f64, height: f64) -> f64 {
        let top = area * scale * scale;
        height / 3.0 * (area + top + (area * top).sqrt())
    }

    const UNIT_SQUARE: &[(f64, f64, f64)] = &[
        (0.0, 0.0, 0.0),
        (1.0, 0.0, 0.0),
        (1.0, 1.0, 0.0),
        (0.0, 1.0, 0.0),
    ];

    // An L (area 3): the non-convex cap case — Voronoi cells are convex, but
    // the contract promises ear-clipped caps for any simple section.
    const L_SHAPE: &[(f64, f64, f64)] = &[
        (0.0, 0.0, 0.0),
        (2.0, 0.0, 0.0),
        (2.0, 1.0, 0.0),
        (1.0, 1.0, 0.0),
        (1.0, 2.0, 0.0),
        (0.0, 2.0, 0.0),
    ];

    #[test]
    fn loft_square_frustum_is_watertight_with_exact_volume() {
        let end = scaled_copy(UNIT_SQUARE, (0.5, 0.5), 0.5, 2.0);
        let mesh = loft(&polygon(UNIT_SQUARE), &end, 64, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.triangle_count(), 2 + 2 + 8, "two caps + 4 quads");
        let want = frustum_volume(1.0, 0.5, 2.0);
        assert!(
            (signed_volume(&mesh) - want).abs() < 1e-12,
            "outward, exact"
        );
    }

    #[test]
    fn loft_orients_outward_when_the_end_is_below_the_start() {
        // Same frustum, end section on the −z side: the signed-volume fix
        // must flip everything, never emit an inside-out solid.
        let end = scaled_copy(UNIT_SQUARE, (0.5, 0.5), 0.5, -2.0);
        let mesh = loft(&polygon(UNIT_SQUARE), &end, 64, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        let want = frustum_volume(1.0, 0.5, 2.0);
        assert!((signed_volume(&mesh) - want).abs() < 1e-12);
        // And a clockwise start section (viewed from +z) lofts the same
        // solid — winding is normalized per section, pairing stays i ↔ i.
        let cw: Vec<(f64, f64, f64)> = UNIT_SQUARE.iter().rev().copied().collect();
        let end_cw = scaled_copy(&cw, (0.5, 0.5), 0.5, 2.0);
        let mesh = loft(&polygon(&cw), &end_cw, 64, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert!((signed_volume(&mesh) - want).abs() < 1e-12);
    }

    #[test]
    fn loft_non_convex_sections_cap_by_ear_clipping() {
        let end = scaled_copy(L_SHAPE, (1.0, 1.0), 0.25, 3.0);
        let mesh = loft(&polygon(L_SHAPE), &end, 64, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert_eq!(
            mesh.triangle_count(),
            4 + 4 + 12,
            "n−2 per cap + 2 per quad"
        );
        let want = frustum_volume(3.0, 0.25, 3.0);
        assert!((signed_volume(&mesh) - want).abs() < 1e-12);
    }

    #[test]
    fn loft_tip_cap_polygon_with_collinear_fused_vertices() {
        // The wall's frusta: a hexagonal cell lofted to a tip-cap polygon
        // with the SAME vertex count whose vertices sit on a triangle —
        // three exactly on its corners, the rest fused 0.03 apart along the
        // edges (tip_caps.py CORNER mode). Collinear cap vertices are legal:
        // zero-area ears, every vertex consumed, boundary edges intact.
        let cell: Vec<(f64, f64, f64)> = (0..6)
            .map(|i| {
                let angle = std::f64::consts::TAU * f64::from(i) / 6.0;
                (10.0 * angle.cos(), 10.0 * angle.sin(), 0.0)
            })
            .collect();
        let corners: [(f64, f64); 3] = [(1.8, 0.0), (-0.9, 1.558_845_7), (-0.9, -1.558_845_7)];
        let mut cap = Vec::new();
        for k in 0..3 {
            let (cx, cy) = corners[k];
            let (nx, ny) = corners[(k + 1) % 3];
            let edge = ((nx - cx).powi(2) + (ny - cy).powi(2)).sqrt();
            cap.push((cx, cy, 20.0));
            cap.push((
                cx + (nx - cx) * 0.03 / edge,
                cy + (ny - cy) * 0.03 / edge,
                20.0,
            ));
        }
        let mesh = loft(&polygon(&cell), &polygon(&cap), 64, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert_eq!(mesh.triangle_count(), 4 + 4 + 12);
        let volume = signed_volume(&mesh);
        // Between the cap's own prism and the cell's: a positive, sane size.
        let cell_area = 6.0 / 2.0 * 100.0 * (std::f64::consts::TAU / 6.0).sin();
        assert!(volume > 0.0 && volume < cell_area * 20.0, "volume {volume}");
    }

    #[test]
    fn loft_circles_make_cones_and_chamfers() {
        let base = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 2.0,
        });
        let top = Curve::Circle(Circle {
            plane: Plane {
                origin: Point::new(0.0, 0.0, 3.0),
                ..world_xy()
            },
            radius: 0.5,
        });
        let mesh = loft(&base, &top, 256, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert_eq!(mesh.vertex_count(), 512);
        // A 256-gon frustum ≈ the cone frustum πh/3 (R² + Rr + r²).
        let want = std::f64::consts::PI * 3.0 / 3.0 * (4.0 + 1.0 + 0.25);
        let got = signed_volume(&mesh);
        assert!((got - want).abs() / want < 1e-3, "got {got}, want ≈ {want}");
        // Seam: both rings start on the plane's +x axis (vertex 0 of each).
        let p = mesh.positions();
        assert!((p[0] - 2.0).abs() < 1e-12 && p[1].abs() < 1e-12);
        let top0 = 256 * 3;
        assert!((p[top0] - 0.5).abs() < 1e-12 && p[top0 + 1].abs() < 1e-12);
    }

    #[test]
    fn loft_rectangles_divide_by_arc_length() {
        // A square divided into 8 hits its corners exactly: the loft of two
        // squares is the exact square frustum. Documented behavior — analytic
        // sections are `segments` arc-length samples, not corner chains.
        let base = Curve::Rectangle(Rectangle {
            plane: world_xy(),
            x: Domain::new(0.0, 2.0),
            y: Domain::new(0.0, 2.0),
        });
        let top = Curve::Rectangle(Rectangle {
            plane: Plane {
                origin: Point::new(0.5, 0.5, 1.0),
                ..world_xy()
            },
            x: Domain::new(0.0, 1.0),
            y: Domain::new(0.0, 1.0),
        });
        let mesh = loft(&base, &top, 8, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert_eq!(mesh.vertex_count(), 16);
        assert!((signed_volume(&mesh) - frustum_volume(4.0, 0.5, 1.0)).abs() < 1e-12);
        // And circle ↔ rectangle pairs by construction: equal `segments`.
        let circle = Curve::Circle(Circle {
            plane: Plane {
                origin: Point::new(1.0, 1.0, 3.0),
                ..world_xy()
            },
            radius: 1.0,
        });
        let mesh = loft(&base, &circle, 8, TOL).expect("lofts");
        assert!(mesh.is_watertight());
        assert!(signed_volume(&mesh) > 0.0);
    }

    #[test]
    fn loft_section_refusals_are_loud() {
        let square = polygon(UNIT_SQUARE);
        // Vertex counts differ — both counts in the message.
        let triangle = polygon(&[(0.0, 0.0, 1.0), (1.0, 0.0, 1.0), (0.0, 1.0, 1.0)]);
        let err = loft(&square, &triangle, 64, TOL).expect_err("refuses");
        assert_eq!(err, LoftError::VertexCountMismatch { start: 4, end: 3 });
        assert!(err.to_string().contains("start has 4, end has 3"));
        // Open section.
        let open = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 1.0),
                Point::new(1.0, 0.0, 1.0),
                Point::new(1.0, 1.0, 1.0),
                Point::new(0.0, 1.0, 1.0),
            ],
            closed: false,
        });
        assert!(matches!(
            loft(&square, &open, 64, TOL),
            Err(LoftError::Section {
                which: "end",
                source: GeomError::OpenCurve { .. }
            })
        ));
        // Degenerate at tolerance: a zero-length edge.
        let pinched = polygon(&[
            (0.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (0.0, 1.0, 1.0),
        ]);
        let err = loft(&square, &pinched, 64, TOL).expect_err("refuses");
        assert!(
            err.to_string().contains("end section") && err.to_string().contains("coincide"),
            "{err}"
        );
        // Collinear (zero-area) section.
        let flat = polygon(&[
            (0.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (2.0, 0.0, 1.0),
            (3.0, 0.0, 1.0),
        ]);
        assert!(matches!(
            loft(&square, &flat, 64, TOL),
            Err(LoftError::Section { which: "end", .. })
        ));
        // Non-planar section.
        let skew = polygon(&[
            (0.0, 0.0, 1.0),
            (1.0, 0.0, 1.0),
            (1.0, 1.0, 1.5),
            (0.0, 1.0, 1.0),
        ]);
        assert!(matches!(
            loft(&square, &skew, 64, TOL),
            Err(LoftError::Section {
                which: "end",
                source: GeomError::NotPlanar { .. }
            })
        ));
        // Self-intersecting section with nonzero area (edges cross, no
        // vertex inside any ear — the post-validation catch).
        let crossing = polygon(&[
            (0.0, 0.0, 1.0),
            (4.0, 0.0, 1.0),
            (1.0, 2.0, 1.0),
            (3.0, 2.0, 1.0),
        ]);
        assert!(matches!(
            loft(&square, &crossing, 64, TOL),
            Err(LoftError::Section {
                which: "end",
                source: GeomError::NotSimple { .. }
            })
        ));
    }

    #[test]
    fn loft_pairing_refusals_are_loud() {
        let square = polygon(UNIT_SQUARE);
        // Coincident sections: zero volume.
        assert!(matches!(
            loft(&square, &square, 64, TOL),
            Err(LoftError::ZeroVolume { .. })
        ));
        // Coplanar but different sections: zero volume too.
        let shifted = scaled_copy(UNIT_SQUARE, (0.5, 0.5), 0.5, 0.0);
        assert!(matches!(
            loft(&square, &shifted, 64, TOL),
            Err(LoftError::ZeroVolume { .. })
        ));
        // Opposite winding: end traversed the other way round.
        let reversed: Vec<(f64, f64, f64)> = UNIT_SQUARE
            .iter()
            .rev()
            .map(|&(x, y, _)| (x, y, 2.0))
            .collect();
        assert_eq!(
            loft(&square, &polygon(&reversed), 64, TOL),
            Err(LoftError::OppositeWinding)
        );
        // Too few segments for an analytic section.
        let circle = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 1.0,
        });
        assert!(matches!(
            loft(&circle, &circle, 2, TOL),
            Err(LoftError::Geom(GeomError::BadParameter {
                name: "segments",
                ..
            }))
        ));
        // Too few vertices.
        let two = polygon(&[(0.0, 0.0, 1.0), (1.0, 0.0, 1.0)]);
        assert!(matches!(
            loft(&two, &two, 64, TOL),
            Err(LoftError::Section { which: "start", .. })
        ));
    }

    proptest::proptest! {
        // Any regular-polygon frustum (homothetic sections, any height,
        // either side): watertight, exact frustum volume, outward.
        #[test]
        fn property_loft_frustum_volume(
            sides in 3usize..16,
            radius in 0.1f64..10.0,
            scale in 0.05f64..3.0,
            height in proptest::prop_oneof![-50.0f64..-0.05, 0.05f64..50.0],
            rotation in 0.0f64..std::f64::consts::TAU,
        ) {
            let base: Vec<(f64, f64, f64)> = (0..sides)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let angle = rotation + std::f64::consts::TAU * i as f64 / sides as f64;
                    (radius * angle.cos(), radius * angle.sin(), 0.0)
                })
                .collect();
            let end = scaled_copy(&base, (0.0, 0.0), scale, height);
            let mesh = loft(&polygon(&base), &end, 64, TOL).expect("lofts");
            proptest::prop_assert!(mesh.is_watertight());
            #[allow(clippy::cast_precision_loss)]
            let area = sides as f64 / 2.0 * radius * radius
                * (std::f64::consts::TAU / sides as f64).sin();
            let want = frustum_volume(area, scale, height.abs());
            let got = signed_volume(&mesh);
            proptest::prop_assert!(
                (got - want).abs() <= 1e-9 * want.max(1.0),
                "got {} want {}", got, want
            );
        }

        // Circle → circle cones: watertight for any segments ≥ 3, volume
        // within the n-gon/circle ratio of the analytic frustum.
        #[test]
        fn property_loft_cone_frustum(
            segments in 3i64..64,
            r0 in 0.1f64..5.0,
            r1 in 0.1f64..5.0,
            height in 0.1f64..10.0,
        ) {
            let base = Curve::Circle(Circle { plane: world_xy(), radius: r0 });
            let top = Curve::Circle(Circle {
                plane: Plane { origin: Point::new(0.0, 0.0, height), ..world_xy() },
                radius: r1,
            });
            let mesh = loft(&base, &top, segments, TOL).expect("lofts");
            proptest::prop_assert!(mesh.is_watertight());
            // Inscribed n-gon area = (n/2π) sin(2π/n) of the disc; the loft's
            // sections are homothetic, so the exact frustum formula applies.
            #[allow(clippy::cast_precision_loss)]
            let ratio = segments as f64 / std::f64::consts::TAU
                * (std::f64::consts::TAU / segments as f64).sin();
            let area = std::f64::consts::PI * r0 * r0 * ratio;
            let want = frustum_volume(area, r1 / r0, height);
            let got = signed_volume(&mesh);
            proptest::prop_assert!(
                (got - want).abs() <= 1e-9 * want.max(1.0),
                "got {} want {}", got, want
            );
        }

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
