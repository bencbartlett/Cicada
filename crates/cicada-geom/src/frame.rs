//! Orthonormal frames from user-supplied planes. The value model stores
//! planes as given (spatial.rs); every constructive operation that needs a
//! trustworthy basis goes through [`orthonormal`] first — degenerate axes
//! are a loud refusal with the reason, never a NaN downstream.

use cicada_core::spatial::{Plane, Point};
use glam::DVec3;

use crate::GeomError;

/// A right-handed orthonormal basis at an origin: `z = x × y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Origin.
    pub origin: Point,
    /// Unit x.
    pub x: DVec3,
    /// Unit y.
    pub y: DVec3,
    /// Unit z (= x × y).
    pub z: DVec3,
}

impl Frame {
    /// Point at plane coordinates `(u, v)` (w = 0).
    #[must_use]
    pub fn point_at(&self, u: f64, v: f64) -> Point {
        Point(self.origin.0 + self.x * u + self.y * v)
    }

    /// Point at full frame coordinates `(u, v, w)`.
    #[must_use]
    pub fn point_at_3(&self, u: f64, v: f64, w: f64) -> Point {
        Point(self.origin.0 + self.x * u + self.y * v + self.z * w)
    }

    /// Frame coordinates of a world point.
    #[must_use]
    pub fn coordinates(&self, point: Point) -> DVec3 {
        let d = point.0 - self.origin.0;
        DVec3::new(d.dot(self.x), d.dot(self.y), d.dot(self.z))
    }
}

/// Gram-Schmidt a user plane into a right-handed orthonormal [`Frame`].
///
/// `tol` guards axis length (a zero-length axis is meaningless at any
/// angle); the parallel check is angular in spirit but expressed through
/// the rejected-component length — a y axis within `tol` of the x line
/// cannot span a plane.
///
/// # Errors
///
/// [`GeomError::DegenerateFrame`] when an axis has no usable length or the
/// axes are parallel.
pub fn orthonormal(plane: &Plane, tol: f64) -> Result<Frame, GeomError> {
    let x_len = plane.x.0.length();
    if crate::tol::near_zero(x_len, tol) {
        return Err(GeomError::DegenerateFrame {
            reason: format!("x axis has length {x_len}"),
        });
    }
    let x = plane.x.0 / x_len;
    let y_raw = plane.y.0;
    let y_rejected = y_raw - x * y_raw.dot(x);
    let y_len = y_rejected.length();
    if crate::tol::near_zero(y_len, tol) {
        return Err(GeomError::DegenerateFrame {
            reason: format!("y axis is parallel to x (rejected component length {y_len})"),
        });
    }
    let y = y_rejected / y_len;
    Ok(Frame {
        origin: plane.origin,
        x,
        y,
        z: x.cross(y),
    })
}

/// A right-handed frame for a polygon loop: origin = vertex centroid,
/// z = Newell normal, x toward the first vertex offset from the centroid.
/// Deterministic for a given loop; refuses degenerate (zero-normal) loops.
///
/// # Errors
///
/// [`GeomError::DegenerateFrame`] when the loop's Newell normal has no
/// usable length (collinear or empty loop).
pub fn polygon_frame(loop_points: &[Point], tol: f64) -> Result<Frame, GeomError> {
    if loop_points.is_empty() {
        return Err(GeomError::DegenerateFrame {
            reason: "empty polygon loop".to_owned(),
        });
    }
    #[allow(clippy::cast_precision_loss)]
    let centroid = loop_points.iter().map(|p| p.0).sum::<DVec3>() / loop_points.len() as f64;
    let mut normal = DVec3::ZERO;
    for (i, a) in loop_points.iter().enumerate() {
        let b = loop_points[(i + 1) % loop_points.len()];
        normal += (a.0 - centroid).cross(b.0 - centroid);
    }
    let len = normal.length();
    if crate::tol::near_zero(len, tol * tol) {
        return Err(GeomError::DegenerateFrame {
            reason: format!("polygon Newell normal has length {len} (collinear loop?)"),
        });
    }
    let z = normal / len;
    // x: the first vertex with a usable offset from the centroid, projected
    // into the plane — deterministic and loop-intrinsic.
    let x_raw = loop_points
        .iter()
        .map(|p| {
            let d = p.0 - centroid;
            d - z * d.dot(z)
        })
        .find(|d| !crate::tol::near_zero(d.length(), tol))
        .ok_or_else(|| GeomError::DegenerateFrame {
            reason: "every vertex is at the centroid".to_owned(),
        })?;
    let x = x_raw.normalize();
    Ok(Frame {
        origin: Point(centroid),
        x,
        y: z.cross(x),
        z,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact identities on unit vectors are the contract here
mod tests {
    use cicada_core::spatial::Vector;

    use super::*;

    const TOL: f64 = 1e-6;

    #[test]
    fn skewed_plane_orthonormalizes() {
        let frame = orthonormal(
            &Plane {
                origin: Point::new(1.0, 2.0, 3.0),
                x: Vector::new(2.0, 0.0, 0.0),
                y: Vector::new(1.0, 1.0, 0.0), // skewed toward x
            },
            TOL,
        )
        .expect("valid plane");
        assert_eq!(frame.x, DVec3::X);
        assert_eq!(frame.y, DVec3::Y);
        assert_eq!(frame.z, DVec3::Z);
    }

    #[test]
    fn degenerate_axes_refuse() {
        let zero_x = Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(0.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        };
        assert!(matches!(
            orthonormal(&zero_x, TOL),
            Err(GeomError::DegenerateFrame { .. })
        ));
        let parallel = Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(2.0, 0.0, 0.0),
        };
        assert!(matches!(
            orthonormal(&parallel, TOL),
            Err(GeomError::DegenerateFrame { .. })
        ));
    }

    #[test]
    fn roundtrip_coordinates() {
        let frame = orthonormal(
            &Plane {
                origin: Point::new(5.0, -2.0, 1.0),
                x: Vector::new(0.0, 3.0, 0.0),
                y: Vector::new(0.0, 0.0, 7.0),
            },
            TOL,
        )
        .expect("valid plane");
        let p = frame.point_at_3(1.5, -2.5, 0.75);
        let back = frame.coordinates(p);
        assert!((back - DVec3::new(1.5, -2.5, 0.75)).length() < 1e-12);
    }
}
