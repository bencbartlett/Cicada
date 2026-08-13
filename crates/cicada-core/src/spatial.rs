//! Spatial value types (doc 14 §Representations): glam f64 types underneath;
//! `Point` and `Vector` are distinct newtypes over `DVec3` — conflating them
//! killed real time on the wall (docs/08).

use glam::{DAffine3, DVec3};

/// A position in space. NOT interchangeable with [`Vector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point(pub DVec3);

/// A displacement/direction. NOT interchangeable with [`Point`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector(pub DVec3);

/// An oriented frame: origin plus x/y axes (z = x × y, derived).
/// Constructors that normalize/orthogonalize live in `cicada-geom`
/// (stage 4); the value model stores what it is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Frame origin.
    pub origin: Point,
    /// X axis.
    pub x: Vector,
    /// Y axis.
    pub y: Vector,
}

/// An affine transform (3×3 linear part + translation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xform(pub DAffine3);

impl Point {
    /// Construct from components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self(DVec3::new(x, y, z))
    }
}

impl Vector {
    /// Construct from components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self(DVec3::new(x, y, z))
    }
}

impl Xform {
    /// The identity transform.
    #[must_use]
    pub const fn identity() -> Self {
        Self(DAffine3::IDENTITY)
    }

    /// The transform's 12 coefficients in canonical hash order:
    /// matrix columns x, y, z then translation, each xyz.
    #[must_use]
    pub fn coefficients(&self) -> [f64; 12] {
        let m = &self.0.matrix3;
        let t = self.0.translation;
        [
            m.x_axis.x, m.x_axis.y, m.x_axis.z, //
            m.y_axis.x, m.y_axis.y, m.y_axis.z, //
            m.z_axis.x, m.z_axis.y, m.z_axis.z, //
            t.x, t.y, t.z,
        ]
    }
}
