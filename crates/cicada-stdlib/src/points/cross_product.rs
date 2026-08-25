//! The `cross_product` node.

use cicada_core::spatial::Vector;
use cicada_macros::node;

use super::VectorPairIn;

/// Cross Product — the vector perpendicular to two vectors, `a × b`
/// (right-handed; its length is `|a|·|b|·sin` of the angle between them).
///
/// # Returns
///
/// The cross product `a × b`.
///
/// # Examples
///
/// ```cic
/// along = construct_vector(x=1.0, y=2.0, z=3.0)
/// across = construct_vector(x=4.0, y=5.0, z=6.0)
/// normal = cross_product(a=along, b=across)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Cross Product"
)]
#[must_use]
pub fn cross_product(input: VectorPairIn) -> Vector {
    Vector(input.a.0.cross(input.b.0))
}

#[cfg(test)]
mod tests {
    use cicada_geom::tol;

    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn cross_product_table() {
        let cross = |a, b| cross_product(VectorPairIn { a, b });
        let close = |a: Vector, b: Vector| tol::near_zero((a.0 - b.0).length(), 1e-12);
        let (x, y, z) = (
            Vector::new(1.0, 0.0, 0.0),
            Vector::new(0.0, 1.0, 0.0),
            Vector::new(0.0, 0.0, 1.0),
        );
        // The right-handed world frame: x × y = z, y × z = x, z × x = y.
        assert!(close(cross(x, y), z));
        assert!(close(cross(y, z), x));
        assert!(close(cross(z, x), y));
        // Anticommutative; a vector with itself (or a parallel one) is zero.
        assert!(close(cross(y, x), Vector::new(0.0, 0.0, -1.0)));
        assert!(close(cross(x, x), Vector::new(0.0, 0.0, 0.0)));
        assert!(close(
            cross(Vector::new(2.0, -4.0, 6.0), Vector::new(-1.0, 2.0, -3.0)),
            Vector::new(0.0, 0.0, 0.0)
        ));
        assert!(close(
            cross(Vector::new(1.0, 2.0, 3.0), Vector::new(4.0, 5.0, 6.0)),
            Vector::new(-3.0, 6.0, -3.0)
        ));
    }

    proptest::proptest! {
        // a × b is perpendicular to both, anticommutative, and has length
        // |a||b| sin θ — which Lagrange's identity states without any trig:
        // |a × b|² + (a · b)² = |a|²|b|². Every one of those holds for b × a
        // too (the C2a review), so the orientation is pinned by the scalar
        // triple product: (a × b) · c is the determinant of [a b c], and a
        // determinant changes sign when two columns swap.
        #[test]
        fn property_cross_product_is_perpendicular(
            ax in -1.0e3..1.0e3_f64, ay in -1.0e3..1.0e3_f64, az in -1.0e3..1.0e3_f64,
            bx in -1.0e3..1.0e3_f64, by in -1.0e3..1.0e3_f64, bz in -1.0e3..1.0e3_f64,
            cx in -1.0e3..1.0e3_f64, cy in -1.0e3..1.0e3_f64, cz in -1.0e3..1.0e3_f64,
        ) {
            let (a, b) = (Vector::new(ax, ay, az), Vector::new(bx, by, bz));
            let c = glam::DVec3::new(cx, cy, cz);
            let ab = cross_product(VectorPairIn { a, b }).0;
            let ba = cross_product(VectorPairIn { a: b, b: a }).0;
            let scale = a.0.length() * b.0.length();
            proptest::prop_assert!(tol::near_zero(ab.dot(a.0), 1e-9 * scale * a.0.length()));
            proptest::prop_assert!(tol::near_zero(ab.dot(b.0), 1e-9 * scale * b.0.length()));
            proptest::prop_assert!(tol::near_zero((ab + ba).length(), 1e-9 * scale));
            let lagrange = ab.length_squared() + a.0.dot(b.0).powi(2);
            proptest::prop_assert!(tol::close(lagrange, scale * scale, 1e-9 * scale * scale));
            // Orientation: the signed volume of the parallelepiped.
            let volume = glam::DMat3::from_cols(a.0, b.0, c).determinant();
            proptest::prop_assert!(tol::close(ab.dot(c), volume, 1e-9 * scale * c.length()));
        }
    }

    // Golden hash: integer components, exact arithmetic (blessed via
    // run-once).
    #[test]
    fn cross_product_determinism_golden_hash() {
        assert_eq!(
            hex(cross_product(VectorPairIn {
                a: Vector::new(1.0, 2.0, 3.0),
                b: Vector::new(4.0, 5.0, 6.0),
            })),
            "54a1ac3180f418368475aad89e34ee865f9f649198c3c0f03dbd2d482a5565e7"
        );
    }
}
