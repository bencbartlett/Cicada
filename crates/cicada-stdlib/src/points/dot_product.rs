//! The `dot_product` node.

use cicada_macros::node;

use super::VectorPairIn;

/// Dot Product — the scalar product of two vectors, `a · b` (`|a|·|b|·cos`
/// of the angle between them: positive when they lean the same way, zero
/// when perpendicular, negative when opposed).
///
/// # Returns
///
/// The dot product `a · b`.
///
/// # Examples
///
/// ```cic
/// along = construct_vector(x=1.0, y=2.0, z=3.0)
/// across = construct_vector(x=4.0, y=5.0, z=6.0)
/// agreement = dot_product(a=along, b=across)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Dot Product"
)]
#[must_use]
pub fn dot_product(input: VectorPairIn) -> f64 {
    input.a.0.dot(input.b.0)
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Vector;
    use cicada_geom::tol;

    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn dot_product_table() {
        let dot = |a, b| dot_product(VectorPairIn { a, b });
        assert!(tol::close(
            dot(Vector::new(1.0, 2.0, 3.0), Vector::new(4.0, 5.0, 6.0)),
            32.0,
            1e-12
        ));
        // Perpendicular: zero; parallel: the product of the lengths;
        // opposed: its negative.
        assert!(tol::near_zero(
            dot(Vector::new(1.0, 0.0, 0.0), Vector::new(0.0, 1.0, 0.0)),
            0.0
        ));
        assert!(tol::close(
            dot(Vector::new(3.0, 4.0, 0.0), Vector::new(3.0, 4.0, 0.0)),
            25.0,
            1e-12
        ));
        assert!(tol::close(
            dot(Vector::new(3.0, 4.0, 0.0), Vector::new(-6.0, -8.0, 0.0)),
            -50.0,
            1e-12
        ));
        assert!(tol::near_zero(
            dot(Vector::new(0.0, 0.0, 0.0), Vector::new(7.0, -1.0, 2.5)),
            0.0
        ));
    }

    proptest::proptest! {
        // Symmetric, bilinear in the first argument, and a · a = |a|².
        #[test]
        fn property_dot_product_is_symmetric_and_bilinear(
            ax in -1.0e3..1.0e3_f64, ay in -1.0e3..1.0e3_f64, az in -1.0e3..1.0e3_f64,
            bx in -1.0e3..1.0e3_f64, by in -1.0e3..1.0e3_f64, bz in -1.0e3..1.0e3_f64,
            s in -10.0..10.0_f64,
        ) {
            let (a, b) = (Vector::new(ax, ay, az), Vector::new(bx, by, bz));
            let dot = |a, b| dot_product(VectorPairIn { a, b });
            let scale = 1.0 + a.0.length() * b.0.length();
            proptest::prop_assert!(tol::close(dot(a, b), dot(b, a), 1e-9 * scale));
            proptest::prop_assert!(tol::close(dot(Vector(a.0 * s), b), s * dot(a, b), 1e-9 * scale * s.abs().max(1.0)));
            proptest::prop_assert!(tol::close(dot(Vector(a.0 + b.0), b), dot(a, b) + dot(b, b), 1e-9 * (scale + b.0.length_squared())));
            proptest::prop_assert!(tol::close(dot(a, a), a.0.length_squared(), 1e-9 * (1.0 + a.0.length_squared())));
        }
    }

    // Golden hash: integer components, exact arithmetic (blessed via
    // run-once).
    #[test]
    fn dot_product_determinism_golden_hash() {
        assert_eq!(
            hex(dot_product(VectorPairIn {
                a: Vector::new(1.0, 2.0, 3.0),
                b: Vector::new(4.0, 5.0, 6.0),
            })),
            "63133244341c9c426aba79b2a82d5a22be705a739d46e289a451686bcbfc72e0"
        );
    }
}
