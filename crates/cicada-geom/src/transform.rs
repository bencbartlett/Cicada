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

    /// Transform any transformable value, preserving its kind (the runtime
    /// half of kind-preserving generics — the checker guarantees the
    /// static half).
    ///
    /// # Panics
    ///
    /// Panics for a [`Transformable::Solid`]: a B-rep transform runs in the
    /// OCCT kernel and lands with v0.1 item 3 WP-C (`SOLID_TRANSFORM_DEFERRED`
    /// is the message). Until then the node goes red with that message —
    /// a loud refusal, never a silent pass-through or a mesh in disguise.
    #[must_use]
    pub fn apply(&self, value: &Transformable) -> Transformable {
        match value {
            Transformable::Point(p) => Transformable::Point(self.apply_point(*p)),
            Transformable::Vector(v) => Transformable::Vector(self.apply_vector(*v)),
            Transformable::Plane(p) => Transformable::Plane(self.apply_plane(p)),
            Transformable::Curve(c) => Transformable::Curve(self.apply_curve(c)),
            Transformable::Mesh(m) => Transformable::Mesh(self.apply_mesh(m)),
            Transformable::Solid(_) => panic!("{SOLID_TRANSFORM_DEFERRED}"),
        }
    }
}

/// The red-node message a transform of a `Solid` carries until the
/// kernel-backed transforms ship (docs/17 Item 3 WP-C).
pub const SOLID_TRANSFORM_DEFERRED: &str = "transforming a Solid (move / rotate / scale / \
     orient / linear_array) is not available yet: B-rep transforms run in the OCCT kernel \
     and arrive with the OCCT-backed solid nodes (v0.1 item 3 WP-C) — until then a Solid \
     cannot be transformed";

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
    fn a_solid_transform_is_a_loud_refusal_until_wp_c() {
        use cicada_core::geometry::{SOLID_CANONICAL_HEADER, Solid};
        let solid = Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).expect("solid");
        let s = Similarity::translation(Vector::new(1.0, 2.0, 3.0));
        let outcome = std::panic::catch_unwind(|| s.apply(&Transformable::Solid(solid)));
        let payload = outcome.expect_err("the transform must refuse, never pass the bytes through");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .expect("a message");
        assert_eq!(message, SOLID_TRANSFORM_DEFERRED);
        assert!(message.contains("not available yet"));
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
