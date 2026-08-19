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

    /// The world origin — the conventional default for plane-constructor
    /// ports (docs/08 `origin: Point = origin`).
    #[must_use]
    pub const fn origin() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }
}

impl Vector {
    /// Construct from components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self(DVec3::new(x, y, z))
    }
}

impl Plane {
    /// The world XY plane at the origin — the conventional default frame
    /// for constructor nodes (docs/08).
    #[must_use]
    pub const fn world_xy() -> Self {
        Self {
            origin: Point::origin(),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 1.0, 0.0),
        }
    }

    /// The world XZ plane at the origin.
    #[must_use]
    pub const fn world_xz() -> Self {
        Self {
            origin: Point::origin(),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(0.0, 0.0, 1.0),
        }
    }

    /// The world YZ plane at the origin.
    #[must_use]
    pub const fn world_yz() -> Self {
        Self {
            origin: Point::origin(),
            x: Vector::new(0.0, 1.0, 0.0),
            y: Vector::new(0.0, 0.0, 1.0),
        }
    }
}

impl Xform {
    /// The identity transform.
    #[must_use]
    pub const fn identity() -> Self {
        Self(DAffine3::IDENTITY)
    }

    /// Rebuild from [`Self::coefficients`] order — the persistence loading
    /// path (keeps glam out of downstream crates).
    #[must_use]
    pub fn from_coefficients(c: [f64; 12]) -> Self {
        Self(DAffine3 {
            matrix3: glam::DMat3::from_cols(
                DVec3::new(c[0], c[1], c[2]),
                DVec3::new(c[3], c[4], c[5]),
                DVec3::new(c[6], c[7], c[8]),
            ),
            translation: DVec3::new(c[9], c[10], c[11]),
        })
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
