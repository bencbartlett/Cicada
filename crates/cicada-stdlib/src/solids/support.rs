//! Test helpers shared by the solid nodes' tests.
//!
//! Golden-hash inputs must stay transcendental-free (boxes and rectangle
//! prisms, never spheres or rotations): sin/cos differ in the last ulp
//! across platform libms, so a transcendental-fed golden would be
//! platform-dependent. Cross-platform kernel identity for curved geometry
//! is measured at stage 6, not here — which is also why `sphere`
//! deliberately has NO mesh golden.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Rectangle};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point};

pub(crate) fn config() -> ProjectConfig {
    ProjectConfig::default()
}

pub(crate) fn unit_square_profile() -> Closed<Curve> {
    Closed(Curve::Rectangle(Rectangle {
        plane: Plane::world_xy(),
        x: Domain::new(0.0, 1.0),
        y: Domain::new(0.0, 1.0),
    }))
}

/// A closed polyline at height `z`: `(x, y)` corners in order.
pub(crate) fn ring(corners: &[(f64, f64)], z: f64) -> Closed<Curve> {
    Closed(Curve::Polyline(cicada_core::geometry::Polyline {
        vertices: corners.iter().map(|&(x, y)| Point::new(x, y, z)).collect(),
        closed: true,
    }))
}

/// Frustum volume between homothetic sections: h/3 · (A₁ + A₂ + √(A₁A₂)).
pub(crate) fn frustum_volume(area: f64, scale: f64, height: f64) -> f64 {
    let top = area * scale * scale;
    height / 3.0 * (area + top + (area * top).sqrt())
}
