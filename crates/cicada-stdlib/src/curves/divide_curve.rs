//! The `divide_curve` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Curve;
use cicada_core::spatial::{Point, Vector};
use cicada_macros::{Ports, node};

use crate::{checked_floor, checked_size, red};

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
/// Panics when `count < 1`; when the slots it would emit over its three
/// outputs — `count + 1` samples on an open curve, `count` on a closed one,
/// each a point, a tangent and a parameter — are above the 2^22 slot
/// ceiling (4,194,304 slots: `count = 1398100` is the last allowed on an
/// open curve, `1398101` on a closed one; the message names `count` and the
/// slot total); or when the curve is degenerate at tolerance (no usable
/// length, zero radius, collapsed frame).
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
    version = 3,
    gh = "Divide Curve",
    uses_tolerance
)]
#[must_use]
pub fn divide_curve(config: &ProjectConfig, input: DivideCurveIn) -> DivideCurveOut {
    // The ceiling is on what the node EMITS: three lists of `samples`
    // slots each, every one hashed by the value model and written to the
    // memo — charged per port, `count = 2^22` was admitted and measured
    // 2.15× the footprint the ceiling is justified by (the review of v0.1
    // follow-up 2). Open/closed is the kernel's own rule (curve.rs module
    // docs); the fattest of the three slots prices the byte half.
    let count = checked_floor("divide_curve", "count", input.count, 1);
    let (samples, each) = if input.curve.is_closed() {
        (count, "count")
    } else {
        (count + 1, "count + 1")
    };
    let _ = checked_size(
        "divide_curve",
        &format!(
            "output slots at count={} ({each} points, tangents and parameters)",
            input.count
        ),
        3 * samples,
        size_of::<Point>()
            .max(size_of::<Vector>())
            .max(size_of::<f64>()),
    );
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

    fn unit_line() -> cicada_core::geometry::Curve {
        line(LineIn {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 0.0, 0.0),
        })
    }

    #[test]
    #[should_panic(expected = "divide_curve: count must be >= 1, got 0")]
    fn divide_curve_zero_count_is_red() {
        let _ = divide_curve(
            &config(),
            DivideCurveIn {
                curve: unit_line(),
                count: 0,
            },
        );
    }

    fn unit_circle() -> cicada_core::geometry::Curve {
        let Closed(curve) = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 1.0,
            },
        );
        curve
    }

    /// The ceiling message for `count` on an open or a closed curve.
    fn refusal(count: i64, closed: bool) -> String {
        let each = if closed { "count" } else { "count + 1" };
        let samples = if closed { count } else { count + 1 };
        format!(
            "divide_curve: output slots at count={count} ({each} points, tangents and \
             parameters) would be {} — above the 4194304 (2^22) slot ceiling of one node output",
            3 * samples
        )
    }

    // The ceiling is on the EMITTED total over the three outputs, at the
    // kernel's own open/closed sample count: on an open curve `count =
    // 1,398,100` emits 3 × 1,398,101 = 4,194,303 slots and builds,
    // `1,398,101` would emit 4,194,306 and is red; a closed curve has no
    // fence-post, so its last allowed count is one higher. These pin where
    // the guard sits and what it says; the absurd case below is what
    // detects a guard moved after the allocation.
    #[test]
    fn divide_curve_emitted_total_at_the_ceiling_builds_and_one_past_it_is_red() {
        let open_last = 1_398_100;
        assert_eq!(3 * (open_last + 1), crate::MAX_SLOTS - 1);
        let at = divide_curve(
            &config(),
            DivideCurveIn {
                curve: unit_line(),
                count: open_last,
            },
        );
        assert_eq!(at.points.len(), 1_398_101);
        assert_eq!(at.tangents.len(), 1_398_101);
        assert_eq!(at.parameters.len(), 1_398_101);
        let past = std::panic::catch_unwind(|| {
            divide_curve(
                &config(),
                DivideCurveIn {
                    curve: unit_line(),
                    count: open_last + 1,
                },
            )
        })
        .expect_err("one past the emitted ceiling refuses");
        assert_eq!(
            past.downcast_ref::<String>().map(String::as_str),
            Some(refusal(open_last + 1, false).as_str())
        );
        assert!(refusal(open_last + 1, false).contains("would be 4194306 —"));

        // Closed: `count` samples each, so `open_last + 1` is the last
        // allowed (4,194,303 slots) and `open_last + 2` is red.
        let at = divide_curve(
            &config(),
            DivideCurveIn {
                curve: unit_circle(),
                count: open_last + 1,
            },
        );
        assert_eq!(at.points.len(), 1_398_101);
        let past = std::panic::catch_unwind(|| {
            divide_curve(
                &config(),
                DivideCurveIn {
                    curve: unit_circle(),
                    count: open_last + 2,
                },
            )
        })
        .expect_err("one past the emitted ceiling refuses on a closed curve too");
        assert_eq!(
            past.downcast_ref::<String>().map(String::as_str),
            Some(refusal(open_last + 2, true).as_str())
        );
    }

    // The absurd count a literal or an Integer wire can carry: 10^11
    // samples is an 800 GB parameter buffer (and 2.4 TB of points) no
    // machine holds — with the guard after the kernel's allocation this
    // test binary would abort on allocation failure (`catch_unwind` cannot
    // catch that), so passing proves the refusal precedes it.
    #[test]
    fn divide_curve_absurd_count_is_refused_not_allocated() {
        let panic = std::panic::catch_unwind(|| {
            divide_curve(
                &config(),
                DivideCurveIn {
                    curve: unit_line(),
                    count: 100_000_000_000,
                },
            )
        })
        .expect_err("an absurd count refuses");
        assert_eq!(
            panic.downcast_ref::<String>().map(String::as_str),
            Some(
                "divide_curve: output slots at count=100000000000 (count + 1 points, tangents \
                 and parameters) would be 300000000003 — above the 4194304 (2^22) slot ceiling \
                 of one node output"
            )
        );
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
