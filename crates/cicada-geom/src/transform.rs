//! Similarity transforms — the exact-on-analytic-curves transform family
//! behind every spike transform node (move, rotate, scale, orient, linear
//! array). A similarity (rigid motion × uniform scale, reflections
//! included) maps circles to circles and rectangles to rectangles, so the
//! analytic value representation survives transformation EXACTLY
//! (DECISIONS.md row 41: curves stay analytic). General affine transforms
//! (which would turn circles into ellipses) deliberately do not exist here.

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle, Transformable};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use glam::{DMat3, DVec3};

use crate::GeomError;
use crate::frame::Frame;

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
        let linear = DMat3::from_axis_angle(frame.z, angle);
        Self {
            linear,
            translation: frame.origin.0 - linear * frame.origin.0,
            scale_abs: 1.0,
            flips: false,
        }
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
}
