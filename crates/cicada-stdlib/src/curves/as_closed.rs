//! The `as_closed` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`as_closed`].
#[derive(Ports, Clone, Debug)]
pub struct AsClosedIn {
    /// The curve to refine.
    pub curve: Curve,
}

/// As Closed — the checked closed-curve refinement (docs/08 rule 5).
/// Already-closed curves pass through unchanged; an open polyline whose
/// endpoints coincide within tolerance closes (duplicate end vertex
/// dropped).
///
/// # Returns
///
/// The curve as a checked closed curve.
///
/// # Panics
///
/// Panics when the curve cannot close: a line, endpoints apart beyond
/// tolerance, or fewer than 3 distinct vertices after closing — red with
/// the distance that failed, never a silent pass (wall lesson 13).
///
/// # Examples
///
/// ```cic
/// xs = [0.0, 4.0, 0.0, 0.0]
/// ys = [0.0, 0.0, 3.0, 0.0]
/// corners = construct_point(x=each(xs), y=each(ys))
/// chain = polyline(vertices=corners)
/// ring = as_closed(curve=chain)
/// ```
#[node(category = "Curve", tier = "S", version = 1, gh = none, uses_tolerance)]
#[must_use]
pub fn as_closed(config: &ProjectConfig, input: AsClosedIn) -> Closed<Curve> {
    Closed(red(cicada_geom::curve::close_curve(
        &input.curve,
        config.tol(),
    )))
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // constructor pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::spatial::Point;

    use crate::curves::polyline::{PolylineIn, polyline};
    use crate::curves::support::config;

    #[test]
    fn as_closed_table() {
        // Closed input passes through unchanged.
        let square = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
            ],
            closed: true,
        });
        let Closed(out) = as_closed(
            &config(),
            AsClosedIn {
                curve: square.clone(),
            },
        );
        assert_eq!(out, square);
        // Coincident-endpoint open polyline closes, dropping the duplicate.
        let nearly = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(0.0, 1.0, 0.0),
                Point::new(0.0, 0.0, 0.0),
            ],
            closed: false,
        });
        let Closed(Curve::Polyline(p)) = as_closed(&config(), AsClosedIn { curve: nearly }) else {
            panic!("stays a polyline")
        };
        assert!(p.closed);
        assert_eq!(p.vertices.len(), 3);
    }

    #[test]
    #[should_panic(expected = "apart")]
    fn as_closed_open_gap_is_red() {
        let open = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(0.0, 1.0, 0.0),
            ],
            closed: false,
        });
        let _ = as_closed(&config(), AsClosedIn { curve: open });
    }

    proptest::proptest! {
        // Any already-closed polyline passes through as_closed unchanged.
        #[test]
        fn property_as_closed_passes_closed_through(
            grid in proptest::collection::hash_set((0u32..50, 0u32..50), 3..12),
        ) {
            let vertices: Vec<Point> = grid
                .iter()
                .map(|&(i, j)| Point::new(f64::from(i), f64::from(j), 0.0))
                .collect();
            let closed = polyline(PolylineIn {
                vertices,
                closed: true,
            });
            let Closed(out) = as_closed(
                &config(),
                AsClosedIn {
                    curve: closed.clone(),
                },
            );
            proptest::prop_assert_eq!(out, closed);
        }
    }

    #[test]
    fn as_closed_determinism_golden_hash() {
        let nearly = polyline(PolylineIn {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(4.0, 0.0, 0.0),
                Point::new(0.0, 3.0, 0.0),
                Point::new(0.0, 0.0, 0.0),
            ],
            closed: false,
        });
        let Closed(out) = as_closed(&config(), AsClosedIn { curve: nearly });
        assert_eq!(
            HashedValue::new(ValueData::Curve(out))
                .unwrap()
                .hash()
                .to_hex(),
            "d12c5b476b2afa66238c98f2f6f649a2ce2b9066928b35557fcac72efa67bd09"
        );
    }
}
