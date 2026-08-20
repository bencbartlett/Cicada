//! The `polyline` node.

use cicada_core::geometry::{Curve, Polyline};
use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`polyline`].
#[derive(Ports, Clone, Debug)]
pub struct PolylineIn {
    /// The vertices, in order.
    pub vertices: Vec<Point>,
    /// Close the chain (implicit edge from the last vertex to the first).
    #[port(default = false)]
    pub closed: bool,
}

/// Polyline — a vertex chain, open or closed.
///
/// # Examples
///
/// ```cic
/// xs = [0.0, 4.0, 4.0, 0.0]
/// ys = [0.0, 0.0, 3.0, 3.0]
/// corners = construct_point(x=each(xs), y=each(ys))
/// outline = polyline(vertices=corners, closed=True)
/// ```
#[node(category = "Curve", tier = "S", version = 1, gh = "PolyLine")]
#[must_use]
pub fn polyline(input: PolylineIn) -> Curve {
    Curve::Polyline(Polyline {
        vertices: input.vertices,
        closed: input.closed,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // constructor pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn polyline_constructs_as_given() {
        let p = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
            ],
            closed: true,
        });
        assert!(p.is_closed());
    }

    proptest::proptest! {
        // Polyline passes any vertex chain and closed flag through exactly.
        #[test]
        fn property_polyline_pass_through(
            coords in proptest::collection::vec(
                (-1.0e6..1.0e6_f64, -1.0e6..1.0e6_f64, -1.0e6..1.0e6_f64),
                0..12,
            ),
            closed in proptest::bool::ANY,
        ) {
            let vertices: Vec<Point> =
                coords.iter().map(|&(x, y, z)| Point::new(x, y, z)).collect();
            let Curve::Polyline(p) = polyline(PolylineIn {
                vertices: vertices.clone(),
                closed,
            }) else {
                panic!("polyline variant")
            };
            proptest::prop_assert_eq!(p.vertices, vertices);
            proptest::prop_assert_eq!(p.closed, closed);
        }
    }

    #[test]
    fn polyline_determinism_golden_hash() {
        let p = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(2.0, 0.0, 0.0),
                Point::new(2.0, 1.5, 0.0),
            ],
            closed: true,
        });
        assert_eq!(
            HashedValue::new(ValueData::Curve(p))
                .unwrap()
                .hash()
                .to_hex(),
            "ddc1f7f597e5931e45efa166b7bc63ea995a15f94ee90aa5f98b88253937cad3"
        );
    }
}
