//! The value-level solid API (v0.1 item 3 WP-B): `core::Solid` in,
//! `core::Solid` out, the same signatures whether or not this build links
//! the OCCT kernel. With the `occt` feature each function reads its
//! inputs' canonical bytes into op-local kernel handles, runs ONE kernel
//! operation and serializes the result back — the sharing model in
//! [`crate::occt`]'s module docs. Without the feature every function
//! returns [`GeomError::KernelUnavailable`]: a loud, typed refusal that
//! names the feature, never a silent fallback to the mesh tier.
//!
//! Callers (the server's display path today, the stdlib's OCCT-backed
//! nodes in WP-C) never see a handle: there is nothing to share, cache or
//! lock at this level. [`kernel_available`] lets a caller or a test say up
//! front which of the two worlds it is in.
//!
//! # The display deflection
//!
//! A solid is drawn through tessellation at a deflection derived from the
//! project's tolerance and unit ([`Deflection::display`]):
//!
//! ```text
//! linear  = max(DISPLAY_DEFLECTION_MM / unit.millimeters(), tol)
//! angular = max(DISPLAY_ANGULAR_RAD, tol_angle)
//! ```
//!
//! [`DISPLAY_DEFLECTION_MM`] is a PHYSICAL chord deviation (0.02 mm) so the
//! same part looks the same in a millimetre, inch or metre document — the
//! unit tag converts it into document units; the `tol` floor says a
//! tessellation finer than the coincidence tolerance is noise (with the
//! default `tol = 1e-6` it never binds). [`DISPLAY_ANGULAR_RAD`] (0.1 rad
//! ≈ 5.7°) gives a full turn ~63 facets — the analytic curves' display
//! tessellation uses 64 segments per circle — and is floored at the
//! angular tolerance for the same reason. The deflection is NOT part of a
//! solid's identity: display tessellations live in the server's
//! hash-keyed display cache, never in the value (docs/12).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Mesh, Solid, Watertight};
use cicada_core::spatial::{Point, Vector};

use crate::GeomError;

/// The physical chord deviation of a display tessellation, in millimetres.
pub const DISPLAY_DEFLECTION_MM: f64 = 0.02;

/// The angular deviation of a display tessellation, in radians.
pub const DISPLAY_ANGULAR_RAD: f64 = 0.1;

/// A tessellation request: absolute linear deflection in document units
/// and angular deflection in radians, both finite and > 0 by construction
/// (so the kernel never sees a deflection it would have to refuse).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Deflection {
    linear: f64,
    angular: f64,
}

impl Deflection {
    /// Validate and construct.
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-finite or non-positive value.
    pub fn new(linear: f64, angular: f64) -> Result<Self, GeomError> {
        for (name, value) in [
            ("linear_deflection", linear),
            ("angular_deflection", angular),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(GeomError::BadParameter {
                    name,
                    value: format!("{value}"),
                    requirement: "must be finite and > 0",
                });
            }
        }
        Ok(Self { linear, angular })
    }

    /// The display deflection for a project — the module docs' formula.
    /// Infallible: `ProjectConfig` holds finite positive tolerances and
    /// the constants are positive.
    #[must_use]
    pub fn display(config: &ProjectConfig) -> Self {
        let linear = (DISPLAY_DEFLECTION_MM / config.unit().millimeters()).max(config.tol());
        let angular = DISPLAY_ANGULAR_RAD.max(config.tol_angle());
        Self { linear, angular }
    }

    /// Absolute linear deflection, document units.
    #[must_use]
    pub fn linear(self) -> f64 {
        self.linear
    }

    /// Angular deflection, radians.
    #[must_use]
    pub fn angular(self) -> f64 {
        self.angular
    }
}

/// What tessellating a solid yields: the welded, watertight mesh and the
/// solid's face count (the display summary's "N faces"), from one
/// reconstruction of the handle.
#[derive(Debug, Clone, PartialEq)]
pub struct Tessellation {
    /// The welded display mesh.
    pub mesh: Watertight<Mesh>,
    /// B-rep faces in the solid.
    pub faces: usize,
}

/// Does this build link the OCCT kernel (`cicada-geom` feature `occt`)?
/// Tests and the display path branch on it; nothing falls back silently.
#[must_use]
pub const fn kernel_available() -> bool {
    cfg!(feature = "occt")
}

#[cfg(feature = "occt")]
mod backend {
    use super::{Deflection, Tessellation};
    use crate::GeomError;
    use crate::occt::Handle;
    use cicada_core::geometry::Solid;
    use cicada_core::spatial::{Point, Vector};

    pub fn box_at(min_corner: Point, extents: Vector) -> Result<Solid, GeomError> {
        Handle::box_at(min_corner, extents)?.into_value()
    }

    pub fn extrude_polygon(
        profile: &[Point],
        direction: Vector,
        tolerance: f64,
    ) -> Result<Solid, GeomError> {
        Handle::extrude_polygon(profile, direction, tolerance)?.into_value()
    }

    pub fn difference(solid: &Solid, cutter: &Solid) -> Result<Solid, GeomError> {
        let solid = Handle::from_value(solid)?;
        let cutter = Handle::from_value(cutter)?;
        solid.difference(cutter)?.into_value()
    }

    pub fn tessellate(solid: &Solid, deflection: Deflection) -> Result<Tessellation, GeomError> {
        Handle::from_value(solid)?.tessellate(deflection)
    }
}

#[cfg(not(feature = "occt"))]
mod backend {
    use super::{Deflection, Tessellation};
    use crate::GeomError;
    use cicada_core::geometry::Solid;
    use cicada_core::spatial::{Point, Vector};

    const fn unavailable(operation: &'static str) -> GeomError {
        GeomError::KernelUnavailable {
            kernel: "OCCT",
            feature: "occt",
            operation,
        }
    }

    pub fn box_at(_min_corner: Point, _extents: Vector) -> Result<Solid, GeomError> {
        Err(unavailable("box"))
    }

    pub fn extrude_polygon(
        _profile: &[Point],
        _direction: Vector,
        _tolerance: f64,
    ) -> Result<Solid, GeomError> {
        Err(unavailable("extrude"))
    }

    pub fn difference(_solid: &Solid, _cutter: &Solid) -> Result<Solid, GeomError> {
        Err(unavailable("difference"))
    }

    pub fn tessellate(_solid: &Solid, _deflection: Deflection) -> Result<Tessellation, GeomError> {
        Err(unavailable("tessellate"))
    }
}

/// An axis-aligned box solid with its minimum corner at `min_corner` and
/// positive `extents` along x, y, z.
///
/// # Errors
///
/// [`GeomError::BadParameter`] for a non-finite or non-positive extent;
/// [`GeomError::Kernel`] / [`GeomError::Serialization`] from the kernel;
/// [`GeomError::KernelUnavailable`] in a build without `occt`.
pub fn box_at(min_corner: Point, extents: Vector) -> Result<Solid, GeomError> {
    backend::box_at(min_corner, extents)
}

/// A closed planar polygon extruded along `direction` into a prism solid;
/// the profile is validated with the mesh tier's rules at `tolerance`
/// (`crate::occt::Handle::extrude_polygon` has the details).
///
/// # Errors
///
/// The profile/direction refusals of the kernel seam;
/// [`GeomError::KernelUnavailable`] in a build without `occt`.
pub fn extrude_polygon(
    profile: &[Point],
    direction: Vector,
    tolerance: f64,
) -> Result<Solid, GeomError> {
    backend::extrude_polygon(profile, direction, tolerance)
}

/// `solid` minus `cutter`. Both inputs are read from their bytes into
/// op-local handles, so the result is the same whatever else happened to
/// these values before (a warm solve equals a cold one).
///
/// # Errors
///
/// [`GeomError::Kernel`] when the cut fails or leaves anything but one
/// solid; [`GeomError::Serialization`] for unreadable bytes;
/// [`GeomError::KernelUnavailable`] in a build without `occt`.
pub fn difference(solid: &Solid, cutter: &Solid) -> Result<Solid, GeomError> {
    backend::difference(solid, cutter)
}

/// Tessellate a solid into a welded, watertight mesh plus its face count.
/// Display callers pass [`Deflection::display`]; the deflection never
/// touches the solid's identity.
///
/// # Errors
///
/// [`GeomError::Kernel`] / [`GeomError::NotWatertight`] from the mesher
/// and the weld; [`GeomError::Serialization`] for unreadable bytes;
/// [`GeomError::KernelUnavailable`] in a build without `occt`.
pub fn tessellate(solid: &Solid, deflection: Deflection) -> Result<Tessellation, GeomError> {
    backend::tessellate(solid, deflection)
}

#[cfg(test)]
mod tests {
    use cicada_core::config::Unit;
    use cicada_core::geometry::SOLID_CANONICAL_HEADER;

    use super::*;

    #[test]
    fn deflection_refuses_bad_values() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                Deflection::new(bad, 0.1),
                Err(GeomError::BadParameter {
                    name: "linear_deflection",
                    ..
                })
            ));
            assert!(matches!(
                Deflection::new(0.1, bad),
                Err(GeomError::BadParameter {
                    name: "angular_deflection",
                    ..
                })
            ));
        }
        let ok = Deflection::new(0.5, 0.25).expect("valid");
        assert!(crate::tol::close(ok.linear(), 0.5, 1e-15));
        assert!(crate::tol::close(ok.angular(), 0.25, 1e-15));
    }

    #[test]
    fn display_deflection_follows_the_documented_formula() {
        // Default project: mm, tol 1e-6 → the physical constants verbatim.
        let mm = Deflection::display(&ProjectConfig::default());
        assert!(crate::tol::close(mm.linear(), DISPLAY_DEFLECTION_MM, 1e-15));
        assert!(crate::tol::close(mm.angular(), DISPLAY_ANGULAR_RAD, 1e-15));
        // Inches: 0.02 mm expressed in inches.
        let inch = Deflection::display(&ProjectConfig::new(Unit::Inch, 1e-6, 1e-9).expect("ok"));
        assert!(crate::tol::close(
            inch.linear(),
            DISPLAY_DEFLECTION_MM / 25.4,
            1e-15
        ));
        // Metres: 2e-5 m — the same physical chord.
        let metre = Deflection::display(&ProjectConfig::new(Unit::Meter, 1e-9, 1e-9).expect("ok"));
        assert!(crate::tol::close(metre.linear(), 2e-5, 1e-18));
        // A coarse tolerance floors the linear deflection; a coarse angular
        // tolerance floors the angular one.
        let coarse =
            Deflection::display(&ProjectConfig::new(Unit::Millimeter, 0.1, 0.3).expect("ok"));
        assert!(crate::tol::close(coarse.linear(), 0.1, 1e-15));
        assert!(crate::tol::close(coarse.angular(), 0.3, 1e-15));
    }

    #[test]
    fn without_the_kernel_every_operation_is_a_typed_refusal() {
        // Both worlds are asserted: this test is honest about which build
        // it runs in and never passes vacuously.
        let bytes = SOLID_CANONICAL_HEADER.to_vec();
        let pseudo = Solid::from_canonical_bytes(bytes).expect("header");
        let deflection = Deflection::display(&ProjectConfig::default());
        if kernel_available() {
            // Real bytes are required by the kernel; a header alone is a
            // serialization error from BinTools, never a crash.
            assert!(matches!(
                tessellate(&pseudo, deflection),
                Err(GeomError::Serialization { .. })
            ));
            assert!(box_at(Point::origin(), Vector::new(1.0, 1.0, 1.0)).is_ok());
        } else {
            let error = tessellate(&pseudo, deflection).expect_err("no kernel");
            assert!(
                matches!(
                    &error,
                    GeomError::KernelUnavailable {
                        kernel: "OCCT",
                        feature: "occt",
                        operation: "tessellate"
                    }
                ),
                "{error}"
            );
            assert!(error.to_string().contains("feature `occt`"), "{error}");
            assert!(matches!(
                box_at(Point::origin(), Vector::new(1.0, 1.0, 1.0)),
                Err(GeomError::KernelUnavailable {
                    operation: "box",
                    ..
                })
            ));
            assert!(matches!(
                difference(&pseudo, &pseudo),
                Err(GeomError::KernelUnavailable {
                    operation: "difference",
                    ..
                })
            ));
            assert!(matches!(
                extrude_polygon(&[], Vector::new(0.0, 0.0, 1.0), 1e-6),
                Err(GeomError::KernelUnavailable {
                    operation: "extrude",
                    ..
                })
            ));
        }
    }
}
