//! Similarity transforms — the exact-on-analytic-curves transform family
//! behind the transform nodes (move, rotate, rotate about an axis, scale,
//! mirror, orient, the arrays). A similarity (rigid motion × uniform
//! scale, reflections included) maps circles to circles and rectangles to
//! rectangles, so the analytic value representation survives
//! transformation EXACTLY (DECISIONS.md row 41: curves stay analytic).
//!
//! General affine transforms exist here too, since catalog C2b
//! (`scale_nu`, `transform` over an arbitrary `Xform`), as [`Affine`] — but
//! with the same promise kept the other way round: an affine that IS a
//! similarity within tolerance is classified as one and applied exactly;
//! one that is not carries only the kinds it carries exactly (points,
//! vectors, lines, polylines, meshes) and REFUSES, typed, the kinds it
//! would have to approximate — a circle whose plane it stretches unevenly
//! would be an ellipse, a skewed frame is not a plane, and the kernel's
//! solid transform takes a similarity (`gp_Trsf`) and nothing else. A
//! circle whose plane is scaled evenly (a z-only stretch of an XY circle)
//! stays a circle, a rectangle aligned with the stretch stays a rectangle
//! with scaled domains: exactness, not similarity, is the rule.

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle, Transformable};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector, Xform};
use glam::{DAffine3, DMat3, DVec3};

use crate::GeomError;
use crate::frame::{Frame, orthonormal};
use crate::tol;

/// A similarity transform: `p ↦ linear·p + translation` with
/// `linear = s·R` (R orthogonal, s ≠ 0; s < 0 = point reflection).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Similarity {
    linear: DMat3,
    translation: DVec3,
    /// |s| — the factor lengths scale by.
    scale_abs: f64,
    /// det(linear) < 0: orientation flips (mesh windings must swap).
    flips: bool,
}

impl Similarity {
    /// Pure translation.
    #[must_use]
    pub fn translation(motion: Vector) -> Self {
        Self {
            linear: DMat3::IDENTITY,
            translation: motion.0,
            scale_abs: 1.0,
            flips: false,
        }
    }

    /// Rotation by `angle` (radians, right-handed) about the frame's z
    /// axis through the frame's origin.
    #[must_use]
    pub fn rotation(frame: &Frame, angle: f64) -> Self {
        Self::rotation_about_axis(frame.origin, frame.z, angle)
    }

    /// Rotation by `angle` (radians, right-handed) about the line through
    /// `origin` along the UNIT direction `axis` — what `rotate` does about a
    /// frame's z and `rotate_axis` about a line (the caller normalizes the
    /// direction and refuses a zero one; this is the one rotation matrix
    /// every rotating node shares, `DMat3::from_axis_angle`).
    #[must_use]
    pub fn rotation_about_axis(origin: Point, axis: DVec3, angle: f64) -> Self {
        let linear = DMat3::from_axis_angle(axis, angle);
        Self {
            linear,
            translation: origin.0 - linear * origin.0,
            scale_abs: 1.0,
            flips: false,
        }
    }

    /// This similarity as the value model's [`Xform`] — the affine
    /// `[linear | translation]` with the classification dropped;
    /// [`Affine::as_similarity`] recovers it.
    #[must_use]
    pub fn xform(&self) -> Xform {
        Xform(DAffine3 {
            matrix3: self.linear,
            translation: self.translation,
        })
    }

    /// Uniform scale about `center` by `factor` (callers refuse factor ≈ 0;
    /// negative factors are point reflections and flip orientation).
    #[must_use]
    pub fn scale_about(center: Point, factor: f64) -> Self {
        let linear = DMat3::IDENTITY * factor;
        Self {
            linear,
            translation: center.0 - linear * center.0,
            scale_abs: factor.abs(),
            // Raw sign test sanctioned: callers refuse |factor| within
            // tolerance of zero (the scale node), so the ambiguous band
            // never reaches this decision (tol discipline, doc 14).
            flips: factor < 0.0,
        }
    }

    /// Reflection across the frame's xy plane (the Householder map
    /// `I − 2·n·nᵀ` about the unit normal `n = frame.z`, through the
    /// frame's origin): an isometry with determinant −1, so lengths are
    /// preserved and orientation flips — mesh windings swap, the kernel
    /// reverses the solid, a plane's derived normal comes out on the
    /// mirrored side. Unlike [`Self::scale_about`] with a negative factor
    /// (a POINT reflection) this fixes every point of the mirror plane.
    #[must_use]
    pub fn reflection(frame: &Frame) -> Self {
        let n = frame.z;
        // Entries `δᵢⱼ − 2·nᵢ·nⱼ`, written as identity minus the scaled
        // outer product so an axis-aligned mirror stays exact arithmetic
        // (the zeros are `0 − 0`, the diagonal `1 − 0` / `1 − 2`).
        let outer = DMat3::from_cols(n * n.x, n * n.y, n * n.z);
        let linear = DMat3::IDENTITY - outer * 2.0;
        Self {
            linear,
            translation: frame.origin.0 - linear * frame.origin.0,
            scale_abs: 1.0,
            flips: true,
        }
    }

    /// The rigid motion carrying `source` onto `target` (both right-handed
    /// orthonormal, so this is always a rotation + translation).
    #[must_use]
    pub fn plane_remap(source: &Frame, target: &Frame) -> Self {
        let source_basis = DMat3::from_cols(source.x, source.y, source.z);
        let target_basis = DMat3::from_cols(target.x, target.y, target.z);
        let linear = target_basis * source_basis.transpose();
        Self {
            linear,
            translation: target.origin.0 - linear * source.origin.0,
            scale_abs: 1.0,
            flips: false,
        }
    }

    /// Transform a position.
    #[must_use]
    pub fn apply_point(&self, p: Point) -> Point {
        Point(self.linear * p.0 + self.translation)
    }

    /// Transform a displacement (linear part only — vectors ignore
    /// translation by definition).
    #[must_use]
    pub fn apply_vector(&self, v: Vector) -> Vector {
        Vector(self.linear * v.0)
    }

    /// Transform a plane axis: direction follows the rotation (and
    /// reflection sign), length is preserved — stored axes keep whatever
    /// length the user gave them.
    fn apply_axis(&self, axis: Vector) -> Vector {
        Vector(self.linear * axis.0 / self.scale_abs)
    }

    fn apply_plane(&self, plane: &Plane) -> Plane {
        Plane {
            origin: self.apply_point(plane.origin),
            x: self.apply_axis(plane.x),
            y: self.apply_axis(plane.y),
        }
    }

    fn apply_curve(&self, curve: &Curve) -> Curve {
        match curve {
            Curve::Line(line) => Curve::Line(Line {
                a: self.apply_point(line.a),
                b: self.apply_point(line.b),
            }),
            Curve::Polyline(polyline) => Curve::Polyline(Polyline {
                vertices: polyline
                    .vertices
                    .iter()
                    .map(|&v| self.apply_point(v))
                    .collect(),
                closed: polyline.closed,
            }),
            Curve::Circle(circle) => Curve::Circle(Circle {
                plane: self.apply_plane(&circle.plane),
                radius: circle.radius * self.scale_abs,
            }),
            Curve::Rectangle(rectangle) => Curve::Rectangle(Rectangle {
                plane: self.apply_plane(&rectangle.plane),
                x: scale_domain(&rectangle.x, self.scale_abs),
                y: scale_domain(&rectangle.y, self.scale_abs),
            }),
        }
    }

    /// Transform a mesh: every position through the affine map; when the
    /// transform flips orientation (negative scale), triangle windings
    /// swap so outward stays outward — booleans downstream depend on it.
    #[must_use]
    pub fn apply_mesh(&self, mesh: &Mesh) -> Mesh {
        let mut positions = Vec::with_capacity(mesh.positions().len());
        let (vertices, _) = mesh.positions().as_chunks::<3>();
        for &[x, y, z] in vertices {
            let p = self.linear * DVec3::new(x, y, z) + self.translation;
            positions.extend_from_slice(&[p.x, p.y, p.z]);
        }
        let indices = if self.flips {
            mesh.indices()
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[a, b, c]| [a, c, b])
                .collect()
        } else {
            mesh.indices().to_vec()
        };
        Mesh::new(positions, indices)
            .unwrap_or_else(|error| unreachable!("transform preserves mesh structure: {error}"))
    }

    /// The 12 row-major coefficients of the 3×4 affine matrix
    /// `[linear | translation]` — what the OCCT kernel's `gp_Trsf` reads
    /// (`crate::solid::transform`).
    #[must_use]
    pub fn coefficients(&self) -> [f64; 12] {
        let r = |i: usize| self.linear.row(i);
        let (r0, r1, r2) = (r(0), r(1), r(2));
        let t = self.translation;
        [
            r0.x, r0.y, r0.z, t.x, //
            r1.x, r1.y, r1.z, t.y, //
            r2.x, r2.y, r2.z, t.z,
        ]
    }

    /// Transform any transformable value, preserving its kind (the runtime
    /// half of kind-preserving generics — the checker guarantees the
    /// static half). A `Solid` goes through the OCCT kernel
    /// (`crate::solid::transform`): the B-rep geometry is rewritten, so the
    /// moved solid's bytes describe the moved geometry — never a mesh in
    /// disguise.
    ///
    /// # Errors
    ///
    /// Only a `Solid` can fail: the kernel's errors, or
    /// [`GeomError::KernelUnavailable`] in a build without the `occt`
    /// feature. Every other kind is total.
    pub fn try_apply(&self, value: &Transformable) -> Result<Transformable, GeomError> {
        Ok(match value {
            Transformable::Point(p) => Transformable::Point(self.apply_point(*p)),
            Transformable::Vector(v) => Transformable::Vector(self.apply_vector(*v)),
            Transformable::Plane(p) => Transformable::Plane(self.apply_plane(p)),
            Transformable::Curve(c) => Transformable::Curve(self.apply_curve(c)),
            Transformable::Mesh(m) => Transformable::Mesh(self.apply_mesh(m)),
            Transformable::Solid(s) => Transformable::Solid(crate::solid::transform(s, self)?),
        })
    }

    /// [`Similarity::try_apply`] for the transform nodes: the one failing
    /// case (a `Solid` the kernel refuses) is a red node.
    ///
    /// # Panics
    ///
    /// Panics with the kernel's message when a `Solid` cannot be
    /// transformed (or the build links no kernel).
    #[must_use]
    pub fn apply(&self, value: &Transformable) -> Transformable {
        match self.try_apply(value) {
            Ok(transformed) => transformed,
            Err(error) => panic!("{error}"),
        }
    }
}

fn scale_domain(domain: &Domain, factor: f64) -> Domain {
    Domain::new(domain.start * factor, domain.end * factor)
}

/// A general affine transform `p ↦ linear·p + translation` — the value
/// model's [`Xform`] with operations on it: composition, the non-uniform
/// scale, classification as a [`Similarity`], and application to a
/// [`Transformable`] that carries only the kinds it carries EXACTLY (the
/// module doc states the rule). Built by `construct_xform` /
/// `compose_xform`, applied by `transform` and `scale_nu`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    linear: DMat3,
    translation: DVec3,
}

impl Affine {
    /// The identity — what `compose_xform` of nothing is.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            linear: DMat3::IDENTITY,
            translation: DVec3::ZERO,
        }
    }

    /// The value model's transform, as is.
    #[must_use]
    pub fn from_xform(xform: &Xform) -> Self {
        Self {
            linear: xform.0.matrix3,
            translation: xform.0.translation,
        }
    }

    /// Back to the value model.
    #[must_use]
    pub fn xform(&self) -> Xform {
        Xform(DAffine3 {
            matrix3: self.linear,
            translation: self.translation,
        })
    }

    /// From the 3×4 matrix written row by row — `[a, b, c, tx, d, e, f,
    /// ty, g, h, i, tz]` maps `(x, y, z)` to `(ax + by + cz + tx, dx + ey +
    /// fz + ty, gx + hy + iz + tz)`; the same order
    /// [`Similarity::coefficients`] emits and the kernel reads.
    #[must_use]
    pub fn from_rows(rows: &[f64; 12]) -> Self {
        Self {
            linear: DMat3::from_cols(
                DVec3::new(rows[0], rows[4], rows[8]),
                DVec3::new(rows[1], rows[5], rows[9]),
                DVec3::new(rows[2], rows[6], rows[10]),
            ),
            translation: DVec3::new(rows[3], rows[7], rows[11]),
        }
    }

    /// The 3×4 matrix row by row — the inverse of [`Self::from_rows`].
    #[must_use]
    pub fn rows(&self) -> [f64; 12] {
        let r = |i: usize| self.linear.row(i);
        let (r0, r1, r2) = (r(0), r(1), r(2));
        let t = self.translation;
        [
            r0.x, r0.y, r0.z, t.x, //
            r1.x, r1.y, r1.z, t.y, //
            r2.x, r2.y, r2.z, t.z,
        ]
    }

    /// Non-uniform scale about the frame's origin by `factors` along the
    /// frame's x, y and z axes: `p ↦ o + B·diag(f)·Bᵀ·(p − o)` with `B` the
    /// frame's basis. Equal factors make it a uniform scale — a similarity
    /// [`Self::as_similarity`] recognizes (exactly, for exactly equal
    /// factors: the matrix is `f·I` to the bit in the world frame). Callers
    /// refuse a factor within tolerance of zero (geometry would collapse).
    #[must_use]
    pub fn scale_in_frame(frame: &Frame, factors: DVec3) -> Self {
        let basis = DMat3::from_cols(frame.x, frame.y, frame.z);
        let linear = basis * DMat3::from_diagonal(factors) * basis.transpose();
        Self {
            linear,
            translation: frame.origin.0 - linear * frame.origin.0,
        }
    }

    /// `next` after `self`: the transform that applies `self` first, then
    /// `next` — `compose_xform` folds a list with it in list order.
    #[must_use]
    pub fn then(&self, next: &Self) -> Self {
        Self {
            linear: next.linear * self.linear,
            translation: next.linear * self.translation + next.translation,
        }
    }

    /// Transform a position.
    #[must_use]
    pub fn apply_point(&self, p: Point) -> Point {
        Point(self.linear * p.0 + self.translation)
    }

    /// Transform a displacement (linear part only).
    #[must_use]
    pub fn apply_vector(&self, v: Vector) -> Vector {
        Vector(self.linear * v.0)
    }

    /// The [`Similarity`] this affine IS, when its linear part is an
    /// orthogonal matrix times one non-zero scale within `tol` — read as a
    /// relative deviation: the three columns' lengths agree with the
    /// first's to `tol` of it and their pairwise dot products vanish to
    /// `tol` of its square. `None` for a stretch (unequal factors), a shear
    /// or a collapse (a first column within `tol` of zero length). The
    /// similarity keeps the matrix AS IS — nothing is re-orthogonalized —
    /// so points transform exactly as the affine would; only the analytic
    /// kinds read the classification (a circle's radius scales by the
    /// first column's length).
    #[must_use]
    pub fn as_similarity(&self, tol: f64) -> Option<Similarity> {
        let columns = [self.linear.x_axis, self.linear.y_axis, self.linear.z_axis];
        let scale = columns[0].length();
        if tol::near_zero(scale, tol) {
            return None;
        }
        let unit = columns.map(|c| c / scale);
        for (i, u) in unit.iter().enumerate() {
            if (u.length() - 1.0).abs() > tol {
                return None;
            }
            for v in &unit[i + 1..] {
                if u.dot(*v).abs() > tol {
                    return None;
                }
            }
        }
        Some(Similarity {
            linear: self.linear,
            translation: self.translation,
            scale_abs: scale,
            // Raw sign test sanctioned: an orthogonal matrix's determinant is
            // ±scale³, far from the ambiguous band once the scale passed the
            // tolerance check above.
            flips: self.linear.determinant() < 0.0,
        })
    }

    /// Transform any transformable value exactly or refuse. A similarity
    /// within `tol` takes [`Similarity::try_apply`] (analytic kinds exact,
    /// a `Solid` through the kernel); otherwise points, vectors, lines,
    /// polylines and meshes transform vertex by vertex (a mesh's windings
    /// swap when the determinant is negative), a plane, a circle and a
    /// rectangle are carried only where the result is still one of their
    /// kind, and a `Solid` is refused.
    ///
    /// # Errors
    ///
    /// [`GeomError::AffineRefused`] for a kind the transform cannot carry
    /// exactly (a circle stretched into an ellipse, a frame skewed, a solid
    /// under a non-similarity); [`GeomError::DegenerateFrame`] for a plane
    /// that was degenerate to begin with; the kernel's errors and
    /// [`GeomError::KernelUnavailable`] for a solid under a similarity.
    pub fn try_apply(&self, value: &Transformable, tol: f64) -> Result<Transformable, GeomError> {
        if let Some(similarity) = self.as_similarity(tol) {
            return similarity.try_apply(value);
        }
        Ok(match value {
            Transformable::Point(p) => Transformable::Point(self.apply_point(*p)),
            Transformable::Vector(v) => Transformable::Vector(self.apply_vector(*v)),
            Transformable::Plane(plane) => Transformable::Plane(self.plane(plane, tol)?.plane),
            Transformable::Curve(curve) => Transformable::Curve(self.curve(curve, tol)?),
            Transformable::Mesh(mesh) => Transformable::Mesh(self.mesh(mesh)),
            Transformable::Solid(_) => {
                return Err(GeomError::AffineRefused {
                    kind: "Solid",
                    reason: "the kernel transform takes a similarity (a rigid motion, a uniform \
                             scale, a reflection) and this one stretches or shears; tessellate \
                             the solid and transform the mesh"
                        .to_owned(),
                });
            }
        })
    }

    fn curve(&self, curve: &Curve, tol: f64) -> Result<Curve, GeomError> {
        Ok(match curve {
            Curve::Line(line) => Curve::Line(Line {
                a: self.apply_point(line.a),
                b: self.apply_point(line.b),
            }),
            Curve::Polyline(polyline) => Curve::Polyline(Polyline {
                vertices: polyline
                    .vertices
                    .iter()
                    .map(|&v| self.apply_point(v))
                    .collect(),
                closed: polyline.closed,
            }),
            Curve::Circle(circle) => {
                let carried = self.plane(&circle.plane, tol)?;
                if (carried.stretch_y - carried.stretch_x).abs() > tol * carried.stretch_x {
                    return Err(GeomError::AffineRefused {
                        kind: "Circle",
                        reason: format!(
                            "its plane is stretched unevenly (x by {}, y by {}) — the result is \
                             an ellipse, which has no kind; scale it uniformly, or turn it into \
                             points with divide_curve first",
                            carried.stretch_x, carried.stretch_y
                        ),
                    });
                }
                Curve::Circle(Circle {
                    plane: carried.plane,
                    radius: circle.radius * carried.stretch_x,
                })
            }
            Curve::Rectangle(rectangle) => {
                let carried = self.plane(&rectangle.plane, tol)?;
                Curve::Rectangle(Rectangle {
                    plane: carried.plane,
                    x: scale_domain(&rectangle.x, carried.stretch_x),
                    y: scale_domain(&rectangle.y, carried.stretch_y),
                })
            }
        })
    }

    /// A plane under the affine: its orthonormal frame's axes follow the
    /// linear part; the result is a plane only while they stay
    /// perpendicular (within `tol`, relative to their lengths) and keep a
    /// length — otherwise the frame is skewed or collapsed and refused.
    /// The stored axes keep the lengths the user gave them (as under a
    /// similarity); the per-axis stretches come back for the circle's
    /// radius and the rectangle's domains.
    fn plane(&self, plane: &Plane, tol: f64) -> Result<CarriedPlane, GeomError> {
        let frame = orthonormal(plane, tol)?;
        let x = self.linear * frame.x;
        let y = self.linear * frame.y;
        let (stretch_x, stretch_y) = (x.length(), y.length());
        if tol::near_zero(stretch_x, tol) || tol::near_zero(stretch_y, tol) {
            return Err(GeomError::AffineRefused {
                kind: "Plane",
                reason: format!(
                    "it collapses an axis of the frame (x stretched by {stretch_x}, y by \
                     {stretch_y}) — a flattened frame is not a plane"
                ),
            });
        }
        let skew = x.dot(y) / (stretch_x * stretch_y);
        if skew.abs() > tol {
            return Err(GeomError::AffineRefused {
                kind: "Plane",
                reason: format!(
                    "it skews the frame (its axes would meet at cos {skew}, not at a right \
                     angle) — a skewed frame is not a plane"
                ),
            });
        }
        Ok(CarriedPlane {
            plane: Plane {
                origin: self.apply_point(plane.origin),
                x: Vector(x / stretch_x * plane.x.0.length()),
                y: Vector(y / stretch_y * plane.y.0.length()),
            },
            stretch_x,
            stretch_y,
        })
    }

    /// Every position through the map; windings swap when the transform
    /// flips orientation, as [`Similarity::apply_mesh`] does.
    #[must_use]
    pub fn mesh(&self, mesh: &Mesh) -> Mesh {
        let mut positions = Vec::with_capacity(mesh.positions().len());
        let (vertices, _) = mesh.positions().as_chunks::<3>();
        for &[x, y, z] in vertices {
            let p = self.linear * DVec3::new(x, y, z) + self.translation;
            positions.extend_from_slice(&[p.x, p.y, p.z]);
        }
        // Raw sign test sanctioned: the determinant's sign IS the orientation
        // (a zero determinant flattens the mesh either way).
        let indices = if self.linear.determinant() < 0.0 {
            mesh.indices()
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|&[a, b, c]| [a, c, b])
                .collect()
        } else {
            mesh.indices().to_vec()
        };
        Mesh::new(positions, indices)
            .unwrap_or_else(|error| unreachable!("transform preserves mesh structure: {error}"))
    }
}

/// A plane carried by an [`Affine`] with the stretch the linear part put
/// on each of its frame axes.
struct CarriedPlane {
    plane: Plane,
    stretch_x: f64,
    stretch_y: f64,
}

#[cfg(test)]
mod tests {
    use crate::frame::orthonormal;
    use crate::meshbuild::{box_mesh, signed_volume};
    use crate::tol;

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
    #[allow(clippy::float_cmp)] // exact: identity rows, a power-of-two scale, integer offsets
    fn coefficients_are_the_row_major_affine_matrix() {
        // Translation: identity rows, the motion in the fourth column.
        let t = Similarity::translation(Vector::new(1.0, 2.0, 3.0));
        assert_eq!(
            t.coefficients(),
            [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, 3.0]
        );
        // Scale about a centre: s on the diagonal, (1 - s)·c in the column.
        let s = Similarity::scale_about(Point::new(1.0, 1.0, 1.0), 2.0);
        assert_eq!(
            s.coefficients(),
            [
                2.0, 0.0, 0.0, -1.0, 0.0, 2.0, 0.0, -1.0, 0.0, 0.0, 2.0, -1.0
            ]
        );
        // The coefficients apply the same map the methods do.
        let p = DVec3::new(0.5, -2.0, 4.0);
        let c = s.coefficients();
        let by_rows = DVec3::new(
            c[0] * p.x + c[1] * p.y + c[2] * p.z + c[3],
            c[4] * p.x + c[5] * p.y + c[6] * p.z + c[7],
            c[8] * p.x + c[9] * p.y + c[10] * p.z + c[11],
        );
        assert!((by_rows - s.apply_point(Point(p)).0).length() < 1e-12);
    }

    #[test]
    fn a_solid_transform_goes_through_the_kernel_or_refuses_loudly() {
        // Both worlds asserted (docs/14): with the kernel a pseudo-solid made
        // of a bare header is a serialization error from BinTools; without
        // it the typed `KernelUnavailable` — and `apply` turns either into
        // a red node carrying that message, never a pass-through.
        use cicada_core::geometry::{SOLID_CANONICAL_HEADER, Solid};
        let solid = Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).expect("solid");
        let s = Similarity::translation(Vector::new(1.0, 2.0, 3.0));
        let error = s
            .try_apply(&Transformable::Solid(solid.clone()))
            .expect_err("a header is not a solid");
        if crate::solid::kernel_available() {
            assert!(matches!(error, GeomError::Serialization { .. }), "{error}");
        } else {
            assert!(
                matches!(
                    error,
                    GeomError::KernelUnavailable {
                        operation: "transform",
                        ..
                    }
                ),
                "{error}"
            );
        }
        let outcome = std::panic::catch_unwind(|| s.apply(&Transformable::Solid(solid)));
        let payload = outcome.expect_err("apply is the red path");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .expect("a message");
        assert_eq!(message, error.to_string());
    }

    #[test]
    fn move_translates_and_leaves_vectors_alone() {
        let s = Similarity::translation(Vector::new(1.0, 2.0, 3.0));
        assert!(tol::coincident(
            s.apply_point(Point::new(0.0, 0.0, 0.0)),
            Point::new(1.0, 2.0, 3.0),
            1e-12
        ));
        let v = s.apply_vector(Vector::new(5.0, 0.0, 0.0));
        assert!((v.0 - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn rotation_quarter_turn_about_offset_center() {
        let center = Plane {
            origin: Point::new(1.0, 0.0, 0.0),
            ..world_xy()
        };
        let frame = orthonormal(&center, TOL).expect("frame");
        let s = Similarity::rotation(&frame, std::f64::consts::FRAC_PI_2);
        let p = s.apply_point(Point::new(2.0, 0.0, 0.0));
        assert!(tol::coincident(p, Point::new(1.0, 1.0, 0.0), 1e-9));
    }

    #[test]
    fn scale_of_circle_stays_a_circle_with_scaled_radius() {
        let s = Similarity::scale_about(Point::new(1.0, 0.0, 0.0), 2.0);
        let circle = Transformable::Curve(Curve::Circle(Circle {
            plane: world_xy(),
            radius: 3.0,
        }));
        let Transformable::Curve(Curve::Circle(out)) = s.apply(&circle) else {
            panic!("kind and variant preserved")
        };
        assert!((out.radius - 6.0).abs() < 1e-12);
        assert!(tol::coincident(
            out.plane.origin,
            Point::new(-1.0, 0.0, 0.0),
            1e-12
        ));
        // Axes keep unit length.
        assert!((out.plane.x.0.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn negative_scale_flips_mesh_winding_keeping_volume_positive() {
        let mesh = box_mesh(
            &world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 2.0),
            Domain::new(0.0, 3.0),
            TOL,
        )
        .expect("builds");
        let s = Similarity::scale_about(Point::new(0.0, 0.0, 0.0), -1.0);
        let reflected = s.apply_mesh(&mesh);
        assert!(reflected.is_watertight());
        assert!(
            (signed_volume(&reflected) - 6.0).abs() < 1e-9,
            "winding swap keeps outward orientation"
        );
    }

    #[test]
    fn reflection_fixes_the_plane_and_flips_the_normal_side() {
        // A tilted mirror through an offset origin: the Householder map
        // about the frame's normal. Points ON the plane stay, a point off
        // it lands at the same distance on the other side, lengths hold,
        // and the transform reports the orientation flip (a reflected box
        // mesh keeps a positive volume through the winding swap).
        let tilted = Plane {
            origin: Point::new(0.0, 0.0, 1.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 1.0),
        };
        let frame = orthonormal(&tilted, TOL).expect("frame");
        let s = Similarity::reflection(&frame);
        let on_plane = Point::new(3.0, 2.0, 3.0); // origin + 3x + 2(y-ish)
        assert!(tol::coincident(s.apply_point(on_plane), on_plane, 1e-12));
        assert!(tol::coincident(
            s.apply_point(frame.origin),
            frame.origin,
            1e-12
        ));
        // The normal's tip mirrors to the other side of the origin.
        let tip = Point(frame.origin.0 + frame.z);
        assert!(tol::coincident(
            s.apply_point(tip),
            Point(frame.origin.0 - frame.z),
            1e-12
        ));
        let a = Point::new(1.0, 5.0, -2.0);
        let b = Point::new(-4.0, 0.5, 7.0);
        let before = (a.0 - b.0).length();
        let after = (s.apply_point(a).0 - s.apply_point(b).0).length();
        assert!((before - after).abs() < 1e-9 * before);
        assert!(s.flips);
        // Coefficients are exact for the axis-aligned mirror (1 − 0, 1 − 2).
        let xy = Similarity::reflection(&orthonormal(&world_xy(), TOL).expect("frame"));
        assert_eq!(
            xy.coefficients().map(f64::to_bits),
            [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0].map(f64::to_bits)
        );
        let mesh = box_mesh(
            &world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 2.0),
            Domain::new(0.0, 3.0),
            TOL,
        )
        .expect("builds");
        let reflected = xy.apply_mesh(&mesh);
        assert!(reflected.is_watertight());
        assert!((signed_volume(&reflected) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn plane_remap_carries_source_onto_target() {
        let source = orthonormal(&world_xy(), TOL).expect("frame");
        let target = orthonormal(
            &Plane {
                origin: Point::new(10.0, 0.0, 0.0),
                x: Vector::new(0.0, 1.0, 0.0),
                y: Vector::new(0.0, 0.0, 1.0),
            },
            TOL,
        )
        .expect("frame");
        let s = Similarity::plane_remap(&source, &target);
        // Source origin lands on target origin; source x lands along
        // target x.
        assert!(tol::coincident(
            s.apply_point(Point::new(0.0, 0.0, 0.0)),
            Point::new(10.0, 0.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            s.apply_point(Point::new(1.0, 0.0, 0.0)),
            Point::new(10.0, 1.0, 0.0),
            1e-12
        ));
    }

    proptest::proptest! {
        // Rigid motions preserve pairwise distance (the defining property).
        #[test]
        fn property_rotation_preserves_distance(
            angle in -6.3f64..6.3,
            ax in -5.0f64..5.0, ay in -5.0f64..5.0,
            bx in -5.0f64..5.0, by in -5.0f64..5.0,
        ) {
            let frame = orthonormal(&world_xy(), TOL).expect("frame");
            let s = Similarity::rotation(&frame, angle);
            let (a, b) = (Point::new(ax, ay, 1.0), Point::new(bx, by, -2.0));
            let before = a.0.distance(b.0);
            let after = s.apply_point(a).0.distance(s.apply_point(b).0);
            proptest::prop_assert!((before - after).abs() <= 1e-9 * before.max(1.0));
        }

        // Uniform scale multiplies distances by |factor| exactly.
        #[test]
        fn property_scale_scales_distance(
            factor in prop_nonzero_factor(),
            ax in -5.0f64..5.0, bx in -5.0f64..5.0,
        ) {
            let s = Similarity::scale_about(Point::new(1.0, 2.0, 3.0), factor);
            let (a, b) = (Point::new(ax, 0.0, 0.0), Point::new(bx, 4.0, 1.0));
            let before = a.0.distance(b.0);
            let after = s.apply_point(a).0.distance(s.apply_point(b).0);
            proptest::prop_assert!(
                (after - before * factor.abs()).abs() <= 1e-9 * (before + 1.0)
            );
        }
    }

    fn prop_nonzero_factor() -> impl proptest::strategy::Strategy<Value = f64> {
        proptest::prop_oneof![0.01f64..10.0, -10.0f64..-0.01]
    }

    // ------------------------------------------------------------ Affine --

    fn tilted_frame() -> Frame {
        orthonormal(
            &Plane {
                origin: Point::new(1.0, -2.0, 0.5),
                x: Vector::new(1.0, 1.0, 0.0),
                y: Vector::new(-1.0, 1.0, 1.0),
            },
            TOL,
        )
        .expect("frame")
    }

    #[test]
    fn rotation_about_an_axis_is_the_frame_rotation_about_its_z() {
        let frame = tilted_frame();
        let by_frame = Similarity::rotation(&frame, 0.7);
        let by_axis = Similarity::rotation_about_axis(frame.origin, frame.z, 0.7);
        assert_eq!(by_frame, by_axis);
        // A quarter turn about the world z line through (1, 0, 0).
        let s = Similarity::rotation_about_axis(
            Point::new(1.0, 0.0, 0.0),
            DVec3::Z,
            std::f64::consts::FRAC_PI_2,
        );
        assert!(tol::coincident(
            s.apply_point(Point::new(2.0, 0.0, 0.0)),
            Point::new(1.0, 1.0, 0.0),
            1e-9
        ));
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact: round trips and identity rows
    fn affine_rows_and_xform_round_trip() {
        let rows = [
            2.0, 0.5, 0.0, 7.0, //
            0.0, 1.0, -3.0, 8.0, //
            0.25, 0.0, 4.0, 9.0,
        ];
        let affine = Affine::from_rows(&rows);
        assert_eq!(affine.rows(), rows);
        assert_eq!(Affine::from_xform(&affine.xform()), affine);
        assert_eq!(Affine::identity().xform(), Xform::identity());
        // A similarity's xform reads back as the same matrix.
        let s = Similarity::scale_about(Point::new(1.0, 1.0, 1.0), 2.0);
        assert_eq!(Affine::from_xform(&s.xform()).rows(), s.coefficients());
        // Rows apply the written map.
        let p = affine.apply_point(Point::new(1.0, 2.0, 3.0));
        assert_eq!(
            p.0,
            DVec3::new(2.0 + 1.0 + 7.0, 2.0 - 9.0 + 8.0, 0.25 + 12.0 + 9.0)
        );
    }

    #[test]
    fn as_similarity_recognizes_every_similarity_and_rejects_stretch_shear_and_collapse() {
        let frame = tilted_frame();
        let similarities = [
            Similarity::translation(Vector::new(1.0, 2.0, 3.0)),
            Similarity::rotation(&frame, 1.1),
            Similarity::scale_about(Point::new(1.0, 0.0, 0.0), 2.5),
            Similarity::scale_about(Point::new(1.0, 0.0, 0.0), -0.5),
            Similarity::reflection(&frame),
            Similarity::plane_remap(&orthonormal(&world_xy(), TOL).expect("frame"), &frame),
        ];
        for s in similarities {
            let back = Affine::from_xform(&s.xform())
                .as_similarity(TOL)
                .unwrap_or_else(|| panic!("{s:?} is a similarity"));
            assert_eq!(back.linear, s.linear);
            assert_eq!(back.translation, s.translation);
            assert!((back.scale_abs - s.scale_abs).abs() <= 1e-12 * s.scale_abs);
            assert_eq!(back.flips, s.flips, "{s:?}");
        }
        // Exactly equal factors: the uniform scale IS a similarity, bit for bit.
        let uniform = Affine::scale_in_frame(
            &orthonormal(&world_xy(), TOL).expect("frame"),
            DVec3::splat(3.0),
        );
        let s = uniform.as_similarity(TOL).expect("uniform");
        assert_eq!(s.linear, DMat3::IDENTITY * 3.0);
        assert!((s.scale_abs - 3.0).abs() < 1e-15);
        // A stretch, a shear and a collapse are not.
        let stretch = Affine::scale_in_frame(&frame, DVec3::new(1.0, 2.0, 1.0));
        assert!(stretch.as_similarity(TOL).is_none());
        let shear = Affine::from_rows(&[
            1.0, 0.5, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ]);
        assert!(shear.as_similarity(TOL).is_none());
        let collapse = Affine::from_rows(&[
            0.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ]);
        assert!(collapse.as_similarity(TOL).is_none());
    }

    #[test]
    fn scale_in_frame_stretches_along_the_frame_axes_about_its_origin() {
        let frame = tilted_frame();
        let affine = Affine::scale_in_frame(&frame, DVec3::new(2.0, 3.0, 0.5));
        // The origin is fixed; a step along each axis is scaled by its factor.
        assert!(tol::coincident(
            affine.apply_point(frame.origin),
            frame.origin,
            1e-12
        ));
        for (axis, factor) in [(frame.x, 2.0), (frame.y, 3.0), (frame.z, 0.5)] {
            let p = Point(frame.origin.0 + axis * 1.5);
            let want = Point(frame.origin.0 + axis * (1.5 * factor));
            assert!(tol::coincident(affine.apply_point(p), want, 1e-9));
        }
    }

    #[test]
    fn affine_carries_a_circle_only_while_its_plane_is_scaled_evenly() {
        let world = orthonormal(&world_xy(), TOL).expect("frame");
        let circle = Transformable::Curve(Curve::Circle(Circle {
            plane: Plane {
                origin: Point::new(1.0, 2.0, 3.0),
                ..world_xy()
            },
            radius: 2.0,
        }));
        // A z-only stretch leaves an XY circle a circle (its plane moves).
        let z_stretch = Affine::scale_in_frame(&world, DVec3::new(1.0, 1.0, 4.0));
        let Transformable::Curve(Curve::Circle(out)) =
            z_stretch.try_apply(&circle, TOL).expect("still a circle")
        else {
            panic!("kind and variant preserved")
        };
        assert!((out.radius - 2.0).abs() < 1e-12);
        assert!(tol::coincident(
            out.plane.origin,
            Point::new(1.0, 2.0, 12.0),
            1e-12
        ));
        // An even in-plane stretch scales the radius.
        let even = Affine::scale_in_frame(&world, DVec3::new(2.0, 2.0, 0.5));
        let Transformable::Curve(Curve::Circle(out)) =
            even.try_apply(&circle, TOL).expect("still a circle")
        else {
            panic!("kind and variant preserved")
        };
        assert!((out.radius - 4.0).abs() < 1e-12);
        // An uneven one is an ellipse: refused, typed, naming the stretches.
        let uneven = Affine::scale_in_frame(&world, DVec3::new(2.0, 1.0, 1.0));
        let error = uneven
            .try_apply(&circle, TOL)
            .expect_err("an ellipse has no kind");
        assert!(
            matches!(&error, GeomError::AffineRefused { kind: "Circle", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("x by 2, y by 1"), "{error}");
        assert!(error.to_string().contains("ellipse"), "{error}");
    }

    #[test]
    fn affine_scales_an_aligned_rectangle_and_refuses_a_skewed_frame() {
        let world = orthonormal(&world_xy(), TOL).expect("frame");
        let rectangle = Transformable::Curve(Curve::Rectangle(Rectangle {
            plane: world_xy(),
            x: Domain::new(-1.0, 3.0),
            y: Domain::new(0.0, 2.0),
        }));
        let stretch = Affine::scale_in_frame(&world, DVec3::new(2.0, 0.5, 7.0));
        let Transformable::Curve(Curve::Rectangle(out)) = stretch
            .try_apply(&rectangle, TOL)
            .expect("still a rectangle")
        else {
            panic!("kind and variant preserved")
        };
        assert!((out.x.start + 2.0).abs() < 1e-12 && (out.x.end - 6.0).abs() < 1e-12);
        assert!(out.y.start.abs() < 1e-12 && (out.y.end - 1.0).abs() < 1e-12);
        // A shear in the rectangle's plane makes a parallelogram: refused.
        let shear = Affine::from_rows(&[
            1.0, 0.5, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ]);
        let error = shear
            .try_apply(&rectangle, TOL)
            .expect_err("a parallelogram is not a rectangle");
        assert!(
            matches!(&error, GeomError::AffineRefused { kind: "Plane", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("skews"), "{error}");
        // The same shear carries a plane whose axes it leaves perpendicular
        // (the YZ plane: the shear moves x only), keeping the axis lengths.
        let yz = Transformable::Plane(Plane {
            origin: Point::new(0.0, 1.0, 0.0),
            x: Vector::new(0.0, 2.0, 0.0),
            y: Vector::new(0.0, 0.0, 3.0),
        });
        let Transformable::Plane(out) = shear.try_apply(&yz, TOL).expect("still a plane") else {
            panic!("kind preserved")
        };
        assert!(tol::coincident(
            out.origin,
            Point::new(0.5, 1.0, 0.0),
            1e-12
        ));
        assert!((out.x.0.length() - 2.0).abs() < 1e-12);
        assert!((out.y.0.length() - 3.0).abs() < 1e-12);
        // A collapse of an axis is refused too.
        let flatten = Affine::from_rows(&[
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ]);
        let error = flatten
            .try_apply(&rectangle, TOL)
            .expect_err("a flattened frame");
        assert!(error.to_string().contains("collapses an axis"), "{error}");
        // Lines and polylines carry under any affine.
        let line = Transformable::Curve(Curve::Line(Line {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(2.0, 2.0, 0.0),
        }));
        let Transformable::Curve(Curve::Line(out)) = shear.try_apply(&line, TOL).expect("a line")
        else {
            panic!("kind and variant preserved")
        };
        assert!(tol::coincident(out.b, Point::new(3.0, 2.0, 0.0), 1e-12));
    }

    // A plane whose stored `y` leans off `x` denotes the frame `orthonormal`
    // makes of it (y's component rejected from x), and that frame is what a
    // non-similarity carries: the axes come back perpendicular — the
    // frame's directions at the stored lengths — while the similarity path
    // moves the stored axes as typed. Pinned so the repair is a contract,
    // not an accident (C2b review).
    #[test]
    fn affine_carries_a_leaning_plane_as_the_frame_it_denotes() {
        let leaning = Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(2.0, 0.0, 0.0),
            // Past a right angle from x (cos < 0): rejected, y is +world y.
            y: Vector::new(-0.5, 3.0, 0.0),
        };
        let y_len = leaning.y.0.length();
        let world = orthonormal(&world_xy(), TOL).expect("frame");
        let stretch = Affine::scale_in_frame(&world, DVec3::new(1.0, 1.0, 4.0));
        let Transformable::Plane(out) = stretch
            .try_apply(&Transformable::Plane(leaning), TOL)
            .expect("a z-stretch carries an XY plane")
        else {
            panic!("kind preserved")
        };
        assert!(out.x.0.dot(out.y.0).abs() < 1e-12, "perpendicular: {out:?}");
        assert!((out.x.0.length() - 2.0).abs() < 1e-12, "{out:?}");
        assert!((out.y.0.length() - y_len).abs() < 1e-12, "{out:?}");
        assert!(
            tol::coincident(Point(out.y.0), Point::new(0.0, y_len, 0.0), 1e-12),
            "y is the rejected direction at its stored length: {out:?}"
        );
        assert!(tol::coincident(
            out.origin,
            Point::new(1.0, 2.0, 12.0),
            1e-12
        ));
        // The similarity path moves the stored axes as typed: still leaning.
        let shift =
            Affine::from_xform(&Similarity::translation(Vector::new(1.0, 0.0, 0.0)).xform());
        let Transformable::Plane(moved) = shift
            .try_apply(&Transformable::Plane(leaning), TOL)
            .expect("a translation carries every plane")
        else {
            panic!("kind preserved")
        };
        assert!(
            tol::coincident(Point(moved.y.0), Point(leaning.y.0), 1e-12),
            "a similarity keeps the axes as typed: {moved:?}"
        );
    }

    #[test]
    fn affine_mesh_winding_follows_the_determinant() {
        let mesh = box_mesh(
            &world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 2.0),
            Domain::new(0.0, 3.0),
            TOL,
        )
        .expect("builds");
        let world = orthonormal(&world_xy(), TOL).expect("frame");
        let stretched = Affine::scale_in_frame(&world, DVec3::new(2.0, 1.0, 1.0));
        let Transformable::Mesh(out) = stretched
            .try_apply(&Transformable::Mesh(mesh.clone()), TOL)
            .expect("a mesh")
        else {
            panic!("kind preserved")
        };
        assert!(out.is_watertight());
        assert!((signed_volume(&out) - 12.0).abs() < 1e-9);
        // One negative factor: the windings swap so the volume stays positive.
        let mirrored = Affine::scale_in_frame(&world, DVec3::new(-2.0, 1.0, 1.0));
        let Transformable::Mesh(out) = mirrored
            .try_apply(&Transformable::Mesh(mesh), TOL)
            .expect("a mesh")
        else {
            panic!("kind preserved")
        };
        assert!(out.is_watertight());
        assert!((signed_volume(&out) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn affine_refuses_a_solid_unless_it_is_a_similarity() {
        use cicada_core::geometry::{SOLID_CANONICAL_HEADER, Solid};
        let solid = Transformable::Solid(
            Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).expect("solid"),
        );
        let world = orthonormal(&world_xy(), TOL).expect("frame");
        // A stretch: refused before any kernel is asked, in both worlds.
        let error = Affine::scale_in_frame(&world, DVec3::new(2.0, 1.0, 1.0))
            .try_apply(&solid, TOL)
            .expect_err("the kernel transform takes a similarity");
        assert!(
            matches!(&error, GeomError::AffineRefused { kind: "Solid", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("tessellate"), "{error}");
        // A similarity takes the kernel path: with the kernel a bare header
        // is a serialization error, without it the typed refusal.
        let error = Affine::scale_in_frame(&world, DVec3::splat(2.0))
            .try_apply(&solid, TOL)
            .expect_err("a header is not a solid");
        if crate::solid::kernel_available() {
            assert!(matches!(error, GeomError::Serialization { .. }), "{error}");
        } else {
            assert!(
                matches!(
                    error,
                    GeomError::KernelUnavailable {
                        operation: "transform",
                        ..
                    }
                ),
                "{error}"
            );
        }
    }

    proptest::proptest! {
        // `a.then(b)` applies a first, then b — for any pair of affines.
        #[test]
        fn property_then_composes_in_order(
            a in proptest::array::uniform12(-3.0f64..3.0),
            b in proptest::array::uniform12(-3.0f64..3.0),
            px in -5.0f64..5.0, py in -5.0f64..5.0, pz in -5.0f64..5.0,
        ) {
            let (a, b) = (Affine::from_rows(&a), Affine::from_rows(&b));
            let p = Point::new(px, py, pz);
            let stepwise = b.apply_point(a.apply_point(p));
            let composed = a.then(&b).apply_point(p);
            proptest::prop_assert!(tol::coincident(stepwise, composed, 1e-9 * stepwise.0.length().max(1.0)));
            let v = Vector::new(px, py, pz);
            let stepwise_v = b.apply_vector(a.apply_vector(v)).0;
            let composed_v = a.then(&b).apply_vector(v).0;
            proptest::prop_assert!((stepwise_v - composed_v).length() <= 1e-9 * stepwise_v.length().max(1.0));
        }

        // A random rotation × scale × translation classifies as a similarity
        // with that scale, and its points transform as the affine's do.
        #[test]
        fn property_similarities_classify_with_their_scale(
            angle in -3.0f64..3.0, factor in prop_nonzero_factor(),
            tx in -5.0f64..5.0, px in -5.0f64..5.0, py in -5.0f64..5.0,
        ) {
            let frame = tilted_frame();
            let rotated = Similarity::rotation(&frame, angle).xform();
            let scaled = Similarity::scale_about(Point::new(tx, 0.0, 1.0), factor).xform();
            let affine = Affine::from_xform(&rotated).then(&Affine::from_xform(&scaled));
            let s = affine.as_similarity(TOL).expect("rotation then scale is a similarity");
            proptest::prop_assert!((s.scale_abs - factor.abs()).abs() <= 1e-9 * factor.abs());
            proptest::prop_assert_eq!(s.flips, factor < 0.0);
            let p = Point::new(px, py, 0.25);
            proptest::prop_assert!(tol::coincident(s.apply_point(p), affine.apply_point(p), 1e-12));
        }
    }
}
