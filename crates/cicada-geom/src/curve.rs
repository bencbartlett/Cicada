//! Curve evaluation: length, division, tessellation, and the `as_closed`
//! conversion. Curves stay analytic in the value model (DECISIONS.md row
//! 41); everything here is a derived computation.
//!
//! Parameterization contract (documented on `divide_curve` in the
//! catalog): `t` is normalized arc length in `0..=1`. Dividing an OPEN
//! curve into `count` segments yields `count + 1` points including both
//! ends; dividing a CLOSED curve yields `count` points (`t = i / count`),
//! the seam point appearing once — the GH Divide Curve convention.

use cicada_core::geometry::{Circle, Curve, Polyline};
use cicada_core::spatial::{Point, Vector};

use crate::frame::{Frame, orthonormal};
use crate::{GeomError, tol};

/// The outputs of curve division, index-aligned.
#[derive(Debug, Clone, PartialEq)]
pub struct Divided {
    /// Sample points.
    pub points: Vec<Point>,
    /// Unit tangents at the samples.
    pub tangents: Vec<Vector>,
    /// Normalized arc-length parameters in `0..=1`.
    pub parameters: Vec<f64>,
}

/// A curve reduced to an evaluable form: an analytic circle, or a vertex
/// chain with tolerance-deduped vertices and cumulative arc lengths.
enum Eval {
    Chain {
        vertices: Vec<Point>,
        closed: bool,
        /// Cumulative length BEFORE segment `i`; `cumulative.len()` =
        /// segment count + 1, last entry = total.
        cumulative: Vec<f64>,
    },
    Circle {
        frame: Frame,
        radius: f64,
    },
}

/// Vertices with consecutive tolerance-coincident points merged (and, for
/// closed chains, the last merged into the first). Evaluation-only — the
/// stored curve is never rewritten.
fn effective_vertices(vertices: &[Point], closed: bool, tolerance: f64) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(vertices.len());
    for &vertex in vertices {
        if out
            .last()
            .is_none_or(|&last| !tol::coincident(last, vertex, tolerance))
        {
            out.push(vertex);
        }
    }
    if closed && out.len() > 1 && tol::coincident(out[0], *out.last().unwrap_or(&out[0]), tolerance)
    {
        out.pop();
    }
    out
}

fn chain(vertices: &[Point], closed: bool, tolerance: f64) -> Result<Eval, GeomError> {
    let vertices = effective_vertices(vertices, closed, tolerance);
    let minimum = if closed { 3 } else { 2 };
    if vertices.len() < minimum {
        return Err(GeomError::DegenerateCurve {
            reason: format!(
                "{} distinct vertices at tolerance {tolerance} (need {minimum})",
                vertices.len()
            ),
        });
    }
    let segment_count = if closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut cumulative = Vec::with_capacity(segment_count + 1);
    cumulative.push(0.0);
    let mut total = 0.0;
    for i in 0..segment_count {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        total += a.0.distance(b.0);
        cumulative.push(total);
    }
    Ok(Eval::Chain {
        vertices,
        closed,
        cumulative,
    })
}

fn eval_form(curve: &Curve, tolerance: f64) -> Result<Eval, GeomError> {
    match curve {
        Curve::Line(line) => chain(&[line.a, line.b], false, tolerance),
        Curve::Polyline(Polyline { vertices, closed }) => chain(vertices, *closed, tolerance),
        Curve::Circle(Circle { plane, radius }) => {
            if *radius <= tolerance {
                return Err(GeomError::DegenerateCurve {
                    reason: format!("circle radius {radius} is not above tolerance {tolerance}"),
                });
            }
            Ok(Eval::Circle {
                frame: orthonormal(plane, tolerance)?,
                radius: *radius,
            })
        }
        Curve::Rectangle(rectangle) => {
            let frame = orthonormal(&rectangle.plane, tolerance)?;
            let (x, y) = (&rectangle.x, &rectangle.y);
            if tol::close(x.start, x.end, tolerance) || tol::close(y.start, y.end, tolerance) {
                return Err(GeomError::DegenerateCurve {
                    reason: format!(
                        "rectangle extent is empty at tolerance {tolerance} \
                         (x {}..{}, y {}..{})",
                        x.start, x.end, y.start, y.end
                    ),
                });
            }
            chain(
                &[
                    frame.point_at(x.start, y.start),
                    frame.point_at(x.end, y.start),
                    frame.point_at(x.end, y.end),
                    frame.point_at(x.start, y.end),
                ],
                true,
                tolerance,
            )
        }
    }
}

/// Arc length of a curve at `tolerance` (used by division; exposed for the
/// tier-1 Length node later).
///
/// # Errors
///
/// [`GeomError::DegenerateCurve`] / [`GeomError::DegenerateFrame`] as in
/// [`divide`].
pub fn length(curve: &Curve, tolerance: f64) -> Result<f64, GeomError> {
    Ok(match eval_form(curve, tolerance)? {
        Eval::Chain { cumulative, .. } => *cumulative.last().unwrap_or(&0.0),
        Eval::Circle { radius, .. } => std::f64::consts::TAU * radius,
    })
}

/// Divide a curve into equal arc-length samples (see the module docs for
/// the open/closed point-count contract).
///
/// # Errors
///
/// [`GeomError::BadParameter`] when `count < 1`;
/// [`GeomError::DegenerateCurve`]/[`GeomError::DegenerateFrame`] when the
/// curve has no usable length or frame at this tolerance.
pub fn divide(curve: &Curve, count: i64, tolerance: f64) -> Result<Divided, GeomError> {
    if count < 1 {
        return Err(GeomError::BadParameter {
            name: "count",
            value: count.to_string(),
            requirement: "must be >= 1",
        });
    }
    let eval = eval_form(curve, tolerance)?;
    let closed = match &eval {
        Eval::Chain { closed, .. } => *closed,
        Eval::Circle { .. } => true,
    };
    let samples = if closed { count } else { count + 1 };
    #[allow(clippy::cast_precision_loss)] // counts are far below 2^53
    let parameters: Vec<f64> = (0..samples).map(|i| i as f64 / count as f64).collect();
    let mut points = Vec::with_capacity(parameters.len());
    let mut tangents = Vec::with_capacity(parameters.len());
    for &t in &parameters {
        let (point, tangent) = sample(&eval, t);
        points.push(point);
        tangents.push(tangent);
    }
    Ok(Divided {
        points,
        tangents,
        parameters,
    })
}

/// Point + unit tangent at normalized arc length `t` (callers guarantee a
/// non-degenerate eval form and `0 <= t <= 1`).
fn sample(eval: &Eval, t: f64) -> (Point, Vector) {
    match eval {
        Eval::Circle { frame, radius } => {
            let angle = std::f64::consts::TAU * t;
            let (sin, cos) = angle.sin_cos();
            let point = frame.point_at(radius * cos, radius * sin);
            let tangent = Vector(frame.x * -sin + frame.y * cos);
            (point, tangent)
        }
        Eval::Chain {
            vertices,
            closed,
            cumulative,
        } => {
            let total = *cumulative.last().unwrap_or(&0.0);
            let s = t * total;
            let segment_count = cumulative.len() - 1;
            // First segment whose END lies STRICTLY beyond s, so a sample
            // landing exactly on a vertex takes the segment LEAVING it (the
            // following segment's tangent). The final sample of an open
            // chain (s = total) finds no such segment and falls to the last
            // one. Vertices are tolerance-deduped, so every segment has
            // usable length.
            let segment = cumulative[1..]
                .iter()
                .position(|&end| end > s)
                .unwrap_or(segment_count - 1)
                .min(segment_count - 1);
            let a = vertices[segment];
            let b = vertices[(segment + 1) % vertices.len()];
            let seg_len = b.0.distance(a.0);
            let local = ((s - cumulative[segment]) / seg_len).clamp(0.0, 1.0);
            let point = Point(a.0.lerp(b.0, local));
            let tangent = Vector((b.0 - a.0) / seg_len);
            // Sampling exactly at a vertex boundary takes the FOLLOWING
            // segment's tangent by the position() rule above — except the
            // open-chain endpoint, which takes the last segment's.
            let _ = closed;
            (point, tangent)
        }
    }
}

/// Tessellate a CLOSED curve into a polygon loop (no duplicate closing
/// vertex). `segments` applies to curved variants (circle); vertex-chain
/// variants return their (tolerance-deduped) corners.
///
/// # Errors
///
/// [`GeomError::OpenCurve`] for open curves; [`GeomError::BadParameter`]
/// when `segments < 3`; degenerate-curve/frame errors as in [`divide`].
pub fn tessellate_closed(
    curve: &Curve,
    segments: i64,
    tolerance: f64,
) -> Result<Vec<Point>, GeomError> {
    if !curve.is_closed() {
        return Err(GeomError::OpenCurve {
            variant: curve.variant_name(),
        });
    }
    if segments < 3 {
        return Err(GeomError::BadParameter {
            name: "segments",
            value: segments.to_string(),
            requirement: "must be >= 3",
        });
    }
    match eval_form(curve, tolerance)? {
        Eval::Chain { vertices, .. } => Ok(vertices),
        eval @ Eval::Circle { .. } => Ok((0..segments)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f64 / segments as f64;
                sample(&eval, t).0
            })
            .collect()),
    }
}

/// The `as_closed` conversion (docs/08: checked refinement, red with the
/// reason on failure): already-closed curves pass through unchanged; an
/// open polyline whose endpoints coincide within tolerance closes by
/// dropping the duplicate end vertex and setting the flag; everything else
/// refuses with the distance that failed.
///
/// # Errors
///
/// [`GeomError::OpenCurve`] for a `Line` (can never close);
/// [`GeomError::Kernel`]-free loud refusal via
/// [`GeomError::DegenerateCurve`] when endpoints are apart or too few
/// vertices remain.
pub fn close_curve(curve: &Curve, tolerance: f64) -> Result<Curve, GeomError> {
    if curve.is_closed() {
        return Ok(curve.clone());
    }
    match curve {
        Curve::Line(_) => Err(GeomError::OpenCurve { variant: "Line" }),
        Curve::Polyline(Polyline { vertices, .. }) => {
            let (Some(&first), Some(&last)) = (vertices.first(), vertices.last()) else {
                return Err(GeomError::DegenerateCurve {
                    reason: "polyline has no vertices".to_owned(),
                });
            };
            if !tol::coincident(first, last, tolerance) {
                return Err(GeomError::DegenerateCurve {
                    reason: format!(
                        "endpoints are {} apart (tolerance {tolerance}); close the \
                         polyline before as_closed",
                        first.0.distance(last.0)
                    ),
                });
            }
            let closed = Polyline {
                vertices: vertices[..vertices.len() - 1].to_vec(),
                closed: true,
            };
            if effective_vertices(&closed.vertices, true, tolerance).len() < 3 {
                return Err(GeomError::DegenerateCurve {
                    reason: format!(
                        "fewer than 3 distinct vertices at tolerance {tolerance} after closing"
                    ),
                });
            }
            Ok(Curve::Polyline(closed))
        }
        Curve::Circle(_) | Curve::Rectangle(_) => unreachable!("already closed"),
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Line, Rectangle};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::Plane;

    use super::*;

    const TOL: f64 = 1e-6;

    fn world_xy() -> Plane {
        Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        }
    }

    fn assert_point_close(got: Point, want: Point) {
        assert!(
            tol::coincident(got, want, 1e-9),
            "point {:?} != {:?}",
            got.0,
            want.0
        );
    }

    #[test]
    fn divide_line_endpoints_and_midpoint() {
        let line = Curve::Line(Line {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(4.0, 0.0, 0.0),
        });
        let out = divide(&line, 2, TOL).expect("divides");
        assert_eq!(out.points.len(), 3, "open curve: count+1 points");
        assert_point_close(out.points[0], Point::new(0.0, 0.0, 0.0));
        assert_point_close(out.points[1], Point::new(2.0, 0.0, 0.0));
        assert_point_close(out.points[2], Point::new(4.0, 0.0, 0.0));
        assert_eq!(out.parameters, vec![0.0, 0.5, 1.0]);
        for tangent in &out.tangents {
            assert!((tangent.0 - glam::DVec3::X).length() < 1e-12);
        }
    }

    #[test]
    fn divide_circle_seam_once() {
        let circle = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 2.0,
        });
        let out = divide(&circle, 4, TOL).expect("divides");
        assert_eq!(out.points.len(), 4, "closed curve: count points");
        assert_point_close(out.points[0], Point::new(2.0, 0.0, 0.0));
        assert_point_close(out.points[1], Point::new(0.0, 2.0, 0.0));
        // Tangent at t=0 is +y for a CCW circle.
        assert!((out.tangents[0].0 - glam::DVec3::Y).length() < 1e-12);
    }

    #[test]
    fn divide_closed_polyline_walks_the_seam_edge() {
        let square = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
                Point::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        });
        let out = divide(&square, 8, TOL).expect("divides");
        assert_eq!(out.points.len(), 8);
        // Sample 7 lies mid-seam-edge (left side, walking down).
        assert_point_close(out.points[7], Point::new(0.0, 0.5, 0.0));
    }

    #[test]
    fn vertex_boundary_samples_take_the_leaving_edges_tangent() {
        // Unit square divided by its vertex count: every sample lands
        // EXACTLY on a vertex, and the tangent must be the edge LEAVING
        // that vertex (the following segment), not the edge arriving.
        let square = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
                Point::new(0.0, 1.0, 0.0),
            ],
            closed: true,
        });
        let Curve::Polyline(p) = &square else {
            panic!("is a polyline")
        };
        let out = divide(&square, 4, TOL).expect("divides");
        assert_eq!(out.points.len(), 4);
        let leaving = [
            glam::DVec3::X,
            glam::DVec3::Y,
            glam::DVec3::NEG_X,
            glam::DVec3::NEG_Y,
        ];
        for (i, (point, tangent)) in out.points.iter().zip(&out.tangents).enumerate() {
            assert_point_close(*point, p.vertices[i]);
            assert!(
                (tangent.0 - leaving[i]).length() < 1e-12,
                "sample {i}: tangent {:?} != leaving edge {:?}",
                tangent.0,
                leaving[i]
            );
        }

        // Open chain: the interior vertex boundary still takes the leaving
        // edge, but the final sample (t = 1) has no following segment and
        // keeps the LAST segment's tangent.
        let corner = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 0.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
            ],
            closed: false,
        });
        let out = divide(&corner, 2, TOL).expect("divides");
        assert_eq!(out.points.len(), 3);
        assert_point_close(out.points[1], Point::new(1.0, 0.0, 0.0));
        assert!(
            (out.tangents[1].0 - glam::DVec3::Y).length() < 1e-12,
            "interior vertex takes the leaving edge"
        );
        assert_point_close(out.points[2], Point::new(1.0, 1.0, 0.0));
        assert!(
            (out.tangents[2].0 - glam::DVec3::Y).length() < 1e-12,
            "t = 1 keeps the last segment's tangent"
        );
    }

    #[test]
    fn divide_refuses_degenerates_loudly() {
        let zero_line = Curve::Line(Line {
            a: Point::new(1.0, 1.0, 1.0),
            b: Point::new(1.0, 1.0, 1.0),
        });
        assert!(matches!(
            divide(&zero_line, 2, TOL),
            Err(GeomError::DegenerateCurve { .. })
        ));
        let circle = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 1.0,
        });
        assert!(matches!(
            divide(&circle, 0, TOL),
            Err(GeomError::BadParameter { name: "count", .. })
        ));
    }

    #[test]
    fn length_of_rectangle_is_perimeter() {
        let rect = Curve::Rectangle(Rectangle {
            plane: world_xy(),
            x: Domain::new(0.0, 3.0),
            y: Domain::new(0.0, 2.0),
        });
        let len = length(&rect, TOL).expect("has length");
        assert!(tol::close(len, 10.0, 1e-9));
    }

    #[test]
    fn tessellate_closed_contract() {
        let circle = Curve::Circle(Circle {
            plane: world_xy(),
            radius: 1.0,
        });
        assert_eq!(tessellate_closed(&circle, 16, TOL).expect("ok").len(), 16);
        let line = Curve::Line(Line {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 0.0, 0.0),
        });
        assert!(matches!(
            tessellate_closed(&line, 16, TOL),
            Err(GeomError::OpenCurve { variant: "Line" })
        ));
        assert!(matches!(
            tessellate_closed(&circle, 2, TOL),
            Err(GeomError::BadParameter { .. })
        ));
    }

    #[test]
    fn close_curve_snaps_within_tolerance_only() {
        let mut vertices = vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(1.0, 0.0, 0.0),
            Point::new(0.0, 1.0, 0.0),
            Point::new(0.0, 5e-7, 0.0), // within 1e-6 of the first vertex
        ];
        let nearly = Curve::Polyline(Polyline {
            vertices: vertices.clone(),
            closed: false,
        });
        let closed = close_curve(&nearly, TOL).expect("closes");
        let Curve::Polyline(p) = &closed else {
            panic!("stays a polyline")
        };
        assert!(p.closed);
        assert_eq!(p.vertices.len(), 3, "duplicate end vertex dropped");

        vertices[3] = Point::new(0.0, 0.5, 0.0); // decisively open
        let open = Curve::Polyline(Polyline {
            vertices,
            closed: false,
        });
        assert!(matches!(
            close_curve(&open, TOL),
            Err(GeomError::DegenerateCurve { .. })
        ));
    }

    proptest::proptest! {
        // Division points of a line are exactly linear interpolations, and
        // parameters are uniform, for any count.
        #[test]
        fn property_line_division_is_linear(
            count in 1i64..40,
            bx in 1.0f64..1e3,
        ) {
            let line = Curve::Line(Line {
                a: Point::new(0.0, 0.0, 0.0),
                b: Point::new(bx, 0.0, 0.0),
            });
            let out = divide(&line, count, TOL).expect("divides");
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let expected = count as usize + 1;
            proptest::prop_assert_eq!(out.points.len(), expected);
            for (i, point) in out.points.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let want = bx * i as f64 / count as f64;
                proptest::prop_assert!((point.0.x - want).abs() < 1e-9 * bx.max(1.0));
            }
        }

        // Circle samples all lie on the circle, tangents unit and
        // perpendicular to the radius.
        #[test]
        fn property_circle_samples_on_circle(
            count in 1i64..64,
            radius in 1e-3f64..1e3,
        ) {
            let circle = Curve::Circle(Circle { plane: world_xy(), radius });
            let out = divide(&circle, count, TOL).expect("divides");
            for (point, tangent) in out.points.iter().zip(&out.tangents) {
                proptest::prop_assert!(
                    (point.0.length() - radius).abs() <= 1e-9 * radius
                );
                proptest::prop_assert!((tangent.0.length() - 1.0).abs() <= 1e-9);
                proptest::prop_assert!(point.0.dot(tangent.0).abs() <= 1e-9 * radius);
            }
        }
    }
}
