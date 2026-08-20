//! The `divide_curve` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Curve;
use cicada_core::spatial::{Point, Vector};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`divide_curve`].
#[derive(Ports, Clone, Debug)]
pub struct DivideCurveIn {
    /// The curve to divide.
    pub curve: Curve,
    /// Number of equal arc-length segments.
    #[port(default = 10)]
    pub count: i64,
}

/// Outputs of [`divide_curve`].
#[derive(Ports, Clone, Debug)]
pub struct DivideCurveOut {
    /// Sample points.
    pub points: Vec<Point>,
    /// Unit tangents at the samples.
    pub tangents: Vec<Vector>,
    /// Normalized arc-length parameters in `0..=1`.
    pub parameters: Vec<f64>,
}

/// Divide Curve — points, tangents, and parameters at equal arc-length
/// steps. An open curve yields `count + 1` samples (both ends included); a
/// closed curve yields `count` (the seam appears once).
///
/// # Panics
///
/// Panics when `count < 1` or the curve is degenerate at tolerance (no
/// usable length, zero radius, collapsed frame).
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// points, tangents, params = divide_curve(curve=ring, count=12)
/// ```
#[node(
    category = "Curve",
    tier = "S",
    version = 1,
    gh = "Divide Curve",
    uses_tolerance
)]
#[must_use]
pub fn divide_curve(config: &ProjectConfig, input: DivideCurveIn) -> DivideCurveOut {
    let divided = red(cicada_geom::curve::divide(
        &input.curve,
        input.count,
        config.tol(),
    ));
    DivideCurveOut {
        points: divided.points,
        tangents: divided.tangents,
        parameters: divided.parameters,
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // constructor pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::geometry::Closed;
    use cicada_core::spatial::Plane;

    use crate::curves::circle::{CircleIn, circle};
    use crate::curves::line::{LineIn, line};
    use crate::curves::support::config;

    #[test]
    fn divide_curve_open_closed_counts() {
        let out = divide_curve(
            &config(),
            DivideCurveIn {
                curve: line(LineIn {
                    a: Point::new(0.0, 0.0, 0.0),
                    b: Point::new(10.0, 0.0, 0.0),
                })
                .clone(),
                count: 4,
            },
        );
        assert_eq!(out.points.len(), 5, "open: count+1");
        assert_eq!(out.parameters, vec![0.0, 0.25, 0.5, 0.75, 1.0]);

        let Closed(circle_curve) = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 1.0,
            },
        );
        let out = divide_curve(
            &config(),
            DivideCurveIn {
                curve: circle_curve,
                count: 4,
            },
        );
        assert_eq!(out.points.len(), 4, "closed: count");
    }

    proptest::proptest! {
        // Dividing any line: points are exact lerps of the endpoints.
        #[test]
        fn property_divide_line_lerps(
            count in 1i64..30,
            len in 0.01f64..1.0e3,
        ) {
            let out = divide_curve(
                &config(),
                DivideCurveIn {
                    curve: line(LineIn {
                        a: Point::new(0.0, 0.0, 0.0),
                        b: Point::new(len, 0.0, 0.0),
                    }),
                    count,
                },
            );
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let want = count as usize + 1;
            proptest::prop_assert_eq!(out.points.len(), want);
            for (point, t) in out.points.iter().zip(&out.parameters) {
                proptest::prop_assert!((point.0.x - t * len).abs() <= 1e-9 * len);
            }
        }
    }

    #[test]
    fn divide_curve_determinism_golden_hash() {
        // Line division is exact lerps + a unit tangent from sqrt(64) —
        // integer-exact arithmetic end to end, no libm.
        let out = divide_curve(
            &config(),
            DivideCurveIn {
                curve: line(LineIn {
                    a: Point::new(0.0, 0.0, 0.0),
                    b: Point::new(8.0, 0.0, 0.0),
                }),
                count: 4,
            },
        );
        let slots = out
            .points
            .into_iter()
            .map(|p| Some(HashedValue::new(ValueData::Point(p)).unwrap()))
            .collect();
        let list = HashedValue::new(ValueData::List(cicada_core::value::List {
            axis: None,
            slots,
        }))
        .unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "f0b0622b350106fbec30122721c4e09b5a6122d281e607371a6f7caa74b973db"
        );
    }
}
