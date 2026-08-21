//! What the transform nodes share: the per-copy payload estimate their
//! allocation guard charges (`linear_array`), and their tests' helpers.

use cicada_core::geometry::{Curve, Transformable};

/// The bytes a fresh copy of `value` allocates beyond its `Transformable`
/// slot — what an array node pays PER SLOT, because every copy is a
/// distinct transformed geometry (`Similarity::apply` builds a new mesh
/// or polyline per copy; nothing is shared, unlike `duplicate`'s
/// `Arc`-shared slots). A mesh is its positions (`f64` each) and indices
/// (`u32` each); a polyline its vertices; the analytic kinds (point,
/// vector, plane, line, circle, rectangle) live inside the enum and add
/// nothing. The guard's `bytes_per_slot` is `size_of::<Transformable>()`
/// plus this, so a 24 MB mesh is refused at a few dozen copies instead of
/// being admitted to millions (v0.1 follow-up 2 review: `linear_array`
/// of a 1,000-segment sphere at `count=100` committed 3.5 GB against a
/// guard that counted 11,200 bytes).
pub(crate) fn payload_bytes(value: &Transformable) -> usize {
    match value {
        Transformable::Mesh(mesh) => size_of_val(mesh.positions()) + size_of_val(mesh.indices()),
        Transformable::Curve(Curve::Polyline(polyline)) => {
            size_of_val(polyline.vertices.as_slice())
        }
        Transformable::Point(_)
        | Transformable::Vector(_)
        | Transformable::Plane(_)
        | Transformable::Curve(Curve::Line(_) | Curve::Circle(_) | Curve::Rectangle(_)) => 0,
    }
}

// Golden-hash inputs stay transcendental-free (docs/14): translation,
// scale, axis-permuted orient, and a ZERO-angle rotation are pure
// arithmetic (sin 0 = 0 and cos 0 = 1 are exact in every libm);
// non-trivial rotation angles would make the hash platform-dependent
// and are forbidden in goldens.
#[cfg(test)]
pub(crate) use testing::*;

#[cfg(test)]
mod testing {
    use cicada_core::config::ProjectConfig;
    use cicada_core::geometry::{Mesh, Transformable};
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};

    pub(crate) fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    pub(crate) fn point(x: f64, y: f64, z: f64) -> Transformable {
        Transformable::Point(Point::new(x, y, z))
    }

    pub(crate) fn expect_point(value: &Transformable) -> Point {
        match value {
            Transformable::Point(p) => *p,
            other => panic!("expected Point, got {other:?}"),
        }
    }

    pub(crate) fn expect_point_hash(value: &Transformable) -> String {
        let Transformable::Point(p) = value else {
            panic!("point stays a point")
        };
        HashedValue::new(ValueData::Point(*p))
            .unwrap()
            .hash()
            .to_hex()
    }

    /// A triangle strip of `vertices` vertices (`vertices - 2` triangles) —
    /// a mesh of a chosen size for the payload tests, built from integer
    /// coordinates in one pass.
    pub(crate) fn strip_mesh(vertices: usize) -> Mesh {
        assert!(vertices >= 3);
        let mut positions = Vec::with_capacity(vertices * 3);
        for i in 0..vertices {
            #[allow(clippy::cast_precision_loss)] // test sizes
            let x = (i / 2) as f64;
            positions.extend_from_slice(&[x, if i % 2 == 0 { 0.0 } else { 1.0 }, 0.0]);
        }
        let indices: Vec<u32> = (0..vertices - 2)
            .flat_map(|i| {
                let i = u32::try_from(i).unwrap();
                if i % 2 == 0 {
                    [i, i + 1, i + 2]
                } else {
                    [i + 1, i, i + 2]
                }
            })
            .collect();
        Mesh::new(positions, indices).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Curve, Polyline, Rectangle};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point, Vector};

    use super::*;

    // The payload is the buffers a copy allocates: a mesh's positions and
    // indices, a polyline's vertices; the analytic kinds add nothing.
    #[test]
    fn payload_bytes_counts_the_buffers_a_copy_allocates() {
        let mesh = strip_mesh(10);
        assert_eq!(mesh.vertex_count(), 10);
        assert_eq!(mesh.triangle_count(), 8);
        assert_eq!(
            payload_bytes(&Transformable::Mesh(mesh)),
            10 * 3 * 8 + 8 * 3 * 4
        );
        let polyline = Polyline {
            vertices: (0..7).map(|i| Point::new(f64::from(i), 0.0, 0.0)).collect(),
            closed: false,
        };
        assert_eq!(
            payload_bytes(&Transformable::Curve(Curve::Polyline(polyline))),
            7 * 24
        );
        for thin in [
            point(1.0, 2.0, 3.0),
            Transformable::Vector(Vector::new(1.0, 0.0, 0.0)),
            Transformable::Plane(Plane::world_xy()),
            Transformable::Curve(Curve::Circle(Circle {
                plane: Plane::world_xy(),
                radius: 1.0,
            })),
            Transformable::Curve(Curve::Rectangle(Rectangle {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
            })),
        ] {
            assert_eq!(payload_bytes(&thin), 0, "{thin:?}");
        }
    }
}
