//! Curve nodes (docs/08 §Catalog 6). Analytic values throughout
//! (DECISIONS.md row 41); evaluation lives in `cicada-geom`.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Circle, Closed, Curve, Line, Polyline, Rectangle};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_geom::frame::orthonormal;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`line`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct LineIn {
    /// Start point.
    pub a: Point,
    /// End point.
    pub b: Point,
}

/// Line — a straight segment between two points.
#[node(category = "Curve", tier = "S", version = 1)]
#[must_use]
pub fn line(input: LineIn) -> Curve {
    Curve::Line(Line {
        a: input.a,
        b: input.b,
    })
}

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
#[node(category = "Curve", tier = "S", version = 1)]
#[must_use]
pub fn polyline(input: PolylineIn) -> Curve {
    Curve::Polyline(Polyline {
        vertices: input.vertices,
        closed: input.closed,
    })
}

/// Inputs for [`circle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct CircleIn {
    /// The circle's frame; origin = center.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The radius.
    #[port(dimension = length)]
    pub radius: f64,
}

/// Circle — an analytic circle in a plane. The stored frame is
/// orthonormalized at construction, so downstream evaluation is exact.
///
/// # Panics
///
/// Panics when the radius is not above tolerance or the plane's axes are
/// degenerate (zero-length or parallel).
#[node(category = "Curve", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn circle(config: &ProjectConfig, input: CircleIn) -> Closed<Curve> {
    assert!(
        input.radius > config.tol(),
        "circle: radius {} is not above tolerance {}",
        input.radius,
        config.tol()
    );
    let frame = red(orthonormal(&input.plane, config.tol()));
    Closed(Curve::Circle(Circle {
        plane: Plane {
            origin: frame.origin,
            x: Vector(frame.x),
            y: Vector(frame.y),
        },
        radius: input.radius,
    }))
}

/// Inputs for [`rectangle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RectangleIn {
    /// The rectangle's frame.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Extent along the plane's x axis.
    pub x: Domain,
    /// Extent along the plane's y axis.
    pub y: Domain,
}

/// Rectangle — an analytic rectangle in a plane, always closed. The frame
/// is orthonormalized at construction. (The rounded-`corner` parameter
/// arrives with compound curves, v0.1.)
///
/// # Panics
///
/// Panics when either extent is empty at tolerance or the plane is
/// degenerate.
#[node(category = "Curve", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn rectangle(config: &ProjectConfig, input: RectangleIn) -> Closed<Curve> {
    for (name, domain) in [("x", &input.x), ("y", &input.y)] {
        assert!(
            !cicada_geom::tol::close(domain.start, domain.end, config.tol()),
            "rectangle: {name} extent {}..{} is empty at tolerance {}",
            domain.start,
            domain.end,
            config.tol()
        );
    }
    let frame = red(orthonormal(&input.plane, config.tol()));
    Closed(Curve::Rectangle(Rectangle {
        plane: Plane {
            origin: frame.origin,
            x: Vector(frame.x),
            y: Vector(frame.y),
        },
        x: input.x,
        y: input.y,
    }))
}

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
#[node(category = "Curve", tier = "S", version = 1, uses_tolerance)]
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
/// # Panics
///
/// Panics when the curve cannot close: a line, endpoints apart beyond
/// tolerance, or fewer than 3 distinct vertices after closing — red with
/// the distance that failed, never a silent pass (wall lesson 13).
#[node(category = "Curve", tier = "S", version = 1, uses_tolerance)]
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

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    #[test]
    fn line_and_polyline_construct_as_given() {
        let l = line(LineIn {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 2.0, 3.0),
        });
        assert!(!l.is_closed());
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

    #[test]
    fn circle_normalizes_its_frame() {
        let skewed = Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(3.0, 0.0, 0.0),
            y: Vector::new(1.0, 2.0, 0.0),
        };
        let Closed(Curve::Circle(c)) = circle(
            &config(),
            CircleIn {
                plane: skewed,
                radius: 2.0,
            },
        ) else {
            panic!("circle variant")
        };
        assert_eq!(c.plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(c.plane.y, Vector::new(0.0, 1.0, 0.0));
        assert_eq!(c.radius, 2.0);
    }

    #[test]
    #[should_panic(expected = "radius")]
    fn circle_zero_radius_is_red() {
        let _ = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 0.0,
            },
        );
    }

    #[test]
    fn rectangle_normalizes_frame_and_keeps_extents() {
        let skewed = Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(2.0, 0.0, 0.0),
            y: Vector::new(1.0, 4.0, 0.0),
        };
        let Closed(Curve::Rectangle(r)) = rectangle(
            &config(),
            RectangleIn {
                plane: skewed,
                x: Domain::new(0.0, 3.0),
                y: Domain::new(-1.0, 2.0),
            },
        ) else {
            panic!("rectangle variant")
        };
        assert_eq!(r.plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(r.plane.y, Vector::new(0.0, 1.0, 0.0));
        assert_eq!(r.x, Domain::new(0.0, 3.0));
        assert_eq!(r.y, Domain::new(-1.0, 2.0));
    }

    #[test]
    #[should_panic(expected = "extent")]
    fn rectangle_empty_extent_is_red() {
        let _ = rectangle(
            &config(),
            RectangleIn {
                plane: Plane::world_xy(),
                x: Domain::new(1.0, 1.0),
                y: Domain::new(0.0, 2.0),
            },
        );
    }

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
        // Line is an exact pass-through constructor for ANY endpoints.
        #[test]
        fn property_line_pass_through(
            ax in -1.0e6..1.0e6_f64, ay in -1.0e6..1.0e6_f64, az in -1.0e6..1.0e6_f64,
            bx in -1.0e6..1.0e6_f64, by in -1.0e6..1.0e6_f64, bz in -1.0e6..1.0e6_f64,
        ) {
            let a = Point::new(ax, ay, az);
            let b = Point::new(bx, by, bz);
            let Curve::Line(l) = line(LineIn { a, b }) else {
                panic!("line variant")
            };
            proptest::prop_assert_eq!(l.a, a);
            proptest::prop_assert_eq!(l.b, b);
        }

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

        // Any non-degenerate skewed frame orthonormalizes: unit orthogonal
        // axes; origin and radius pass through exactly.
        #[test]
        fn property_circle_orthonormalizes_any_frame(
            ox in -1.0e3..1.0e3_f64, oy in -1.0e3..1.0e3_f64,
            xa in 0.5..10.0_f64, xb in -5.0..5.0_f64,
            yc in -5.0..5.0_f64, yd in 0.5..10.0_f64,
            radius in 0.001..1.0e3_f64,
        ) {
            // Keep the axes clearly non-parallel so the frame is far from
            // degenerate (near-parallel Gram-Schmidt loses precision).
            proptest::prop_assume!((xa * yd - xb * yc).abs() > 0.5);
            let plane = Plane {
                origin: Point::new(ox, oy, 0.0),
                x: Vector::new(xa, xb, 0.0),
                y: Vector::new(yc, yd, 0.0),
            };
            let Closed(Curve::Circle(c)) = circle(&config(), CircleIn { plane, radius })
            else {
                panic!("circle variant")
            };
            proptest::prop_assert!((c.plane.x.0.length() - 1.0).abs() <= 1e-12);
            proptest::prop_assert!((c.plane.y.0.length() - 1.0).abs() <= 1e-12);
            proptest::prop_assert!(c.plane.x.0.dot(c.plane.y.0).abs() <= 1e-9);
            proptest::prop_assert_eq!(c.plane.origin, plane.origin);
            proptest::prop_assert_eq!(c.radius, radius);
        }

        // Rectangle keeps its extents exactly for any non-empty domains.
        #[test]
        fn property_rectangle_keeps_domains(
            x0 in -100.0..100.0_f64, dx in 0.01..50.0_f64,
            y0 in -100.0..100.0_f64, dy in 0.01..50.0_f64,
        ) {
            let Closed(Curve::Rectangle(r)) = rectangle(
                &config(),
                RectangleIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(x0, x0 + dx),
                    y: Domain::new(y0, y0 + dy),
                },
            ) else {
                panic!("rectangle variant")
            };
            proptest::prop_assert_eq!(r.x, Domain::new(x0, x0 + dx));
            proptest::prop_assert_eq!(r.y, Domain::new(y0, y0 + dy));
        }

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

    // Golden-hash inputs stay transcendental-free (docs/14): sin/cos differ
    // in the last ulp across platform libms, so goldens are built from
    // lines/polylines/rectangles (pure arithmetic). The circle golden is
    // fine: it hashes the analytic VALUE (plane + radius, no evaluation);
    // dividing a circle would not be.

    #[test]
    fn line_determinism_golden_hash() {
        let l = line(LineIn {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 2.0, 3.0),
        });
        assert_eq!(
            HashedValue::new(ValueData::Curve(l))
                .unwrap()
                .hash()
                .to_hex(),
            "d25432f6a628adba13074041192cdae076447dfa6b6d3a1ea798919662167107"
        );
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

    #[test]
    fn circle_determinism_golden_hash() {
        let Closed(c) = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 2.5,
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Curve(c))
                .unwrap()
                .hash()
                .to_hex(),
            "49e447cfea5876a978743c331b265d0c8c1824a052e96e9fc8386023e06dffd3"
        );
    }

    #[test]
    fn rectangle_determinism_golden_hash() {
        let Closed(r) = rectangle(
            &config(),
            RectangleIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 3.0),
                y: Domain::new(-1.0, 2.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Curve(r))
                .unwrap()
                .hash()
                .to_hex(),
            "fe8f0016efa7cd5bc86f6f28a0f7e03158e5da9f67235740a7b65de4e773592c"
        );
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
