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
//! project's tolerance and unit, in two tiers — the FINE tier
//! ([`Deflection::display`]) for structural generations and the viewport
//! at rest, the PREVIEW tier ([`Deflection::preview`]) for the
//! generations of a slider drag, which the release refines:
//!
//! ```text
//! fine:     linear  = max(DISPLAY_DEFLECTION_MM / unit.millimeters(), tol)
//!           angular = max(DISPLAY_ANGULAR_RAD, tol_angle)
//! preview:  linear  = max(PREVIEW_DEFLECTION_MM / unit.millimeters(), tol)
//!           angular = max(PREVIEW_ANGULAR_RAD, tol_angle)
//! per solid (both tiers, tessellate_display):
//!           linear  = max(linear, DISPLAY_RELATIVE × max extent of the solid's bounds)
//! ```
//!
//! [`DISPLAY_DEFLECTION_MM`] is a PHYSICAL chord deviation (0.02 mm) so the
//! same part looks the same in a millimetre, inch or metre document — the
//! unit tag converts it into document units; the `tol` floor says a
//! tessellation finer than the coincidence tolerance is noise (with the
//! default `tol = 1e-6` it never binds). [`DISPLAY_ANGULAR_RAD`] (0.1 rad
//! ≈ 5.7°) gives a full turn ~63 facets — the analytic curves' display
//! tessellation uses 64 segments per circle — and is floored at the
//! angular tolerance for the same reason. The preview tier
//! ([`PREVIEW_DEFLECTION_MM`] 0.1 mm, [`PREVIEW_ANGULAR_RAD`] 0.3 rad ≈ 21
//! facets per turn) is what a drag can afford: measured on the 02-solids
//! carve, 3 ms against 23 ms at the fine tier (docs/17 §Item 3). The
//! relative term ([`DISPLAY_RELATIVE`], 1/1000 of the part's largest
//! extent — OCCT's own viewer convention, `Prs3d_Drawer`'s deviation
//! coefficient) keeps a giant smooth part from drowning in triangles: a
//! 2 m sphere at 0.02 mm would need ~700 facets per turn; at 2 mm the
//! angular term decides and it gets the same ~63 as a small one. The
//! deflection is NOT part of a solid's identity: display tessellations
//! live in the server's hash-keyed display cache, never in the value
//! (docs/12).
//!
//! Closure is the NODE's contract, not display's: [`tessellate`] returns
//! `Watertight<Mesh>` or refuses, because the mesh tier needs a closed
//! mesh; [`tessellate_display`] returns the welded mesh whether or not it
//! closed and says which ([`DisplayTessellation::watertight`]), because a
//! green Solid must never vanish from the viewport — the kernel's mesher
//! can hand back non-conforming face triangulations for a valid solid
//! (the moved-sphere-minus-cylinder regression in `occt/tests.rs`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Mesh, Solid, Watertight};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use glam::{DVec2, DVec3};

use crate::curve::{WireForm, wire_form};
use crate::frame::{Frame, orthonormal, polygon_frame};
use crate::transform::Similarity;
use crate::triangulate::ear_clip;
use crate::{GeomError, tol};

/// The physical chord deviation of a display tessellation, in millimetres.
pub const DISPLAY_DEFLECTION_MM: f64 = 0.02;

/// The angular deviation of a display tessellation, in radians.
pub const DISPLAY_ANGULAR_RAD: f64 = 0.1;

/// The physical chord deviation of a PREVIEW tessellation (the generations
/// of a slider drag), in millimetres.
pub const PREVIEW_DEFLECTION_MM: f64 = 0.1;

/// The angular deviation of a preview tessellation, in radians (~21 facets
/// per full turn).
pub const PREVIEW_ANGULAR_RAD: f64 = 0.3;

/// The relative term of a display tessellation: the linear deflection is
/// at least this fraction of the solid's largest bounding-box extent
/// (OCCT's viewer uses the same 0.001 as its deviation coefficient).
pub const DISPLAY_RELATIVE: f64 = 0.001;

/// The finest linear deflection a [`Deflection`] admits: OCCT's
/// `Precision::Confusion()` (1e-7). `BRepMesh_IncrementalMesh` throws
/// `Standard_NumericError` ("invalid parameter value") for anything finer,
/// so the floor is the kernel's, restated here as a typed refusal at
/// construction (the seam's tests drive the raw glue below it to prove the
/// floor is necessary, and the mesher at exactly the floor to prove it is
/// sufficient).
pub const MIN_LINEAR_DEFLECTION: f64 = 1e-7;

/// The finest angular deflection a [`Deflection`] admits: OCCT's
/// `Precision::Angular()` (1e-12 rad), the mesher's other floor.
pub const MIN_ANGULAR_DEFLECTION: f64 = 1e-12;

/// A tessellation request: absolute linear deflection in document units
/// and angular deflection in radians, both finite and at or above the
/// kernel's floors ([`MIN_LINEAR_DEFLECTION`], [`MIN_ANGULAR_DEFLECTION`])
/// by construction — so a deflection the kernel would refuse is refused
/// here first, with a typed error that names the floor.
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
    /// [`GeomError::BadParameter`] for a non-finite value, or one below the
    /// kernel's floor (`linear < 1e-7`, `angular < 1e-12`; zero and
    /// negatives included).
    pub fn new(linear: f64, angular: f64) -> Result<Self, GeomError> {
        if !(linear.is_finite() && linear >= MIN_LINEAR_DEFLECTION) {
            return Err(GeomError::BadParameter {
                name: "linear_deflection",
                value: format!("{linear}"),
                requirement: "must be finite and >= 1e-7 (OCCT Precision::Confusion — the \
                              mesher refuses anything finer)",
            });
        }
        if !(angular.is_finite() && angular >= MIN_ANGULAR_DEFLECTION) {
            return Err(GeomError::BadParameter {
                name: "angular_deflection",
                value: format!("{angular}"),
                requirement: "must be finite and >= 1e-12 rad (OCCT Precision::Angular — the \
                              mesher refuses anything finer)",
            });
        }
        Ok(Self { linear, angular })
    }

    /// The display deflection for a project — the module docs' formula.
    /// Infallible: `ProjectConfig` holds finite positive tolerances, the
    /// constants are positive, and the formula's minimum over every `Unit`
    /// (0.02 mm in a foot document is 6.6e-5) sits well above the kernel's
    /// floor — `display_deflection_is_above_the_floor_for_every_unit` pins
    /// that, so this constructor can never build a value `new` would
    /// refuse.
    #[must_use]
    pub fn display(config: &ProjectConfig) -> Self {
        let linear = (DISPLAY_DEFLECTION_MM / config.unit().millimeters()).max(config.tol());
        let angular = DISPLAY_ANGULAR_RAD.max(config.tol_angle());
        Self { linear, angular }
    }

    /// The preview-tier display deflection for a project — the module
    /// docs' formula with the preview constants. Infallible for the same
    /// reason as [`Deflection::display`] (coarser on both axes).
    #[must_use]
    pub fn preview(config: &ProjectConfig) -> Self {
        let linear = (PREVIEW_DEFLECTION_MM / config.unit().millimeters()).max(config.tol());
        let angular = PREVIEW_ANGULAR_RAD.max(config.tol_angle());
        Self { linear, angular }
    }

    /// The deflection for drawing a solid whose largest bounding-box extent
    /// is `extent`: the linear deflection raised to [`DISPLAY_RELATIVE`] ×
    /// `extent` when that is coarser (the module docs' relative term); the
    /// angular deflection is unchanged. A non-finite or non-positive extent
    /// leaves the deflection as it is.
    #[must_use]
    pub fn for_extent(self, extent: f64) -> Self {
        if !(extent.is_finite() && extent > 0.0) {
            return self;
        }
        Self {
            linear: self.linear.max(DISPLAY_RELATIVE * extent),
            angular: self.angular,
        }
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

/// What the `tessellate` node yields: the welded, watertight mesh and the
/// solid's face count, from one reconstruction of the handle.
#[derive(Debug, Clone, PartialEq)]
pub struct Tessellation {
    /// The welded, closed mesh.
    pub mesh: Watertight<Mesh>,
    /// B-rep faces in the solid.
    pub faces: usize,
}

/// What drawing a solid yields ([`tessellate_display`]): the welded mesh,
/// whether it closed, the face count (the summary's "N faces") and the
/// deflection it was actually meshed at (the tier's, raised by the
/// relative term for this solid's extent).
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayTessellation {
    /// The welded display mesh — drawn whether or not it is closed.
    pub mesh: Mesh,
    /// Did the weld close it? `false` is reported in the summary, never
    /// hidden; it says the mesher's per-face triangulations did not
    /// conform along some edge, not that the solid is invalid.
    pub watertight: bool,
    /// B-rep faces in the solid.
    pub faces: usize,
    /// The deflection the mesher ran at.
    pub deflection: Deflection,
}

/// Does this build link the OCCT kernel (`cicada-geom` feature `occt`)?
/// Tests and the display path branch on it; nothing falls back silently.
#[must_use]
pub const fn kernel_available() -> bool {
    cfg!(feature = "occt")
}

#[cfg(feature = "occt")]
mod backend {
    use super::{Deflection, DisplayTessellation, Tessellation};
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

    pub fn tessellate_display(
        solid: &Solid,
        tier: Deflection,
    ) -> Result<DisplayTessellation, GeomError> {
        let handle = Handle::from_value(solid)?;
        // The relative term needs the solid's extent: one bounds read on
        // the handle about to be meshed (microseconds next to the mesher).
        let (min, max) = handle.bounds()?;
        let extent = (max.0 - min.0).max_element();
        handle.tessellate_display(tier.for_extent(extent))
    }

    pub fn is_valid(solid: &Solid) -> Result<bool, GeomError> {
        Handle::from_value(solid)?.is_valid()
    }
}

#[cfg(not(feature = "occt"))]
mod backend {
    use super::{Deflection, DisplayTessellation, Tessellation};
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

    pub fn tessellate_display(
        _solid: &Solid,
        _tier: Deflection,
    ) -> Result<DisplayTessellation, GeomError> {
        Err(unavailable("display"))
    }

    pub fn is_valid(_solid: &Solid) -> Result<bool, GeomError> {
        Err(unavailable("is_valid"))
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

/// Tessellate a solid for DISPLAY at a tier's deflection
/// ([`Deflection::display`] or [`Deflection::preview`]), raised by the
/// relative term for this solid's extent (the module docs' formula): the
/// welded mesh whether or not it closed, with `watertight` saying which,
/// the face count, and the deflection the mesher ran at. One
/// reconstruction of the handle serves the bounds and the mesh.
///
/// # Errors
///
/// [`GeomError::Kernel`] from the mesher or the bounds;
/// [`GeomError::Serialization`] for unreadable bytes;
/// [`GeomError::KernelUnavailable`] in a build without `occt`. Never
/// `NotWatertight` — closure is reported, not required.
pub fn tessellate_display(
    solid: &Solid,
    tier: Deflection,
) -> Result<DisplayTessellation, GeomError> {
    backend::tessellate_display(solid, tier)
}

/// `BRepCheck_Analyzer`'s verdict on a solid — the kernel's own validity
/// check (topology and geometry). Diagnostic: the tests use it to tell an
/// invalid boolean result from a valid solid whose mesh does not close.
///
/// # Errors
///
/// The kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn is_valid(solid: &Solid) -> Result<bool, GeomError> {
    backend::is_valid(solid)
}

// ---------------------------------------------------------------------------
// The node set (v0.1 item 3 WP-C)
// ---------------------------------------------------------------------------

/// The fixed `FILE_NAME.time_stamp` every STEP file [`write_step`] writes:
/// byte-determinism of an export is worth more than a wall-clock the
/// file's consumers never read.
pub const STEP_TIMESTAMP: &str = "2000-01-01T00:00:00";

/// What [`volume`] measures: the enclosed volume (document units³) and the
/// volume centroid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeProperties {
    /// The enclosed volume.
    pub volume: f64,
    /// The volume centroid.
    pub centroid: Point,
}

/// A validated closed planar profile: its wire form and the plane it lies
/// in (origin, unit axes). Every sweep constructor starts here, so the
/// refusals — open, degenerate, non-planar, self-intersecting — are the
/// mesh tier's, word for word, before the kernel sees anything (OCCT
/// accepts collinear points and returns a zero-volume solid; it is not the
/// validator).
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The wire to build.
    pub form: WireForm,
    /// The profile's plane: the polygon's Newell frame, or the circle's.
    pub frame: Frame,
}

impl Profile {
    /// Validate a closed planar profile at `tolerance`.
    ///
    /// # Errors
    ///
    /// [`GeomError::OpenCurve`] for an open curve; [`GeomError::NotPlanar`],
    /// [`GeomError::NotSimple`], [`GeomError::DegenerateCurve`] /
    /// [`GeomError::DegenerateFrame`] for a bad loop.
    pub fn closed(curve: &Curve, tolerance: f64) -> Result<Self, GeomError> {
        if !curve.is_closed() {
            return Err(GeomError::OpenCurve {
                variant: curve.variant_name(),
            });
        }
        let form = wire_form(curve, tolerance)?;
        let frame = match &form {
            WireForm::Chain { vertices, .. } => {
                let frame = polygon_frame(vertices, tolerance)?;
                let mut flat = Vec::with_capacity(vertices.len());
                for (vertex, point) in vertices.iter().enumerate() {
                    let local = frame.coordinates(*point);
                    if !tol::near_zero(local.z, tolerance) {
                        return Err(GeomError::NotPlanar {
                            vertex,
                            distance: local.z,
                        });
                    }
                    flat.push(DVec2::new(local.x, local.y));
                }
                // Simplicity + non-zero area, with the mesh tier's refusals;
                // the triangles themselves are not needed.
                ear_clip(&flat, tolerance)?;
                frame
            }
            WireForm::Circle { frame, .. } => *frame,
        };
        Ok(Self { form, frame })
    }

    /// The points of the profile's plane that `direction` must leave:
    /// refuses a direction within `tolerance` of lying in the plane.
    fn require_leaving(&self, direction: Vector, tolerance: f64) -> Result<(), GeomError> {
        if !direction.0.is_finite() || tol::near_zero(direction.0.dot(self.frame.z), tolerance) {
            return Err(GeomError::BadParameter {
                name: "direction",
                value: format!("{:?}", direction.0),
                requirement: "must be finite and leave the profile plane (not be parallel to it)",
            });
        }
        Ok(())
    }
}

/// The wire form of a sweep rail: any curve with usable length, open or
/// closed (a line, a polyline, a circle, a rectangle's corners).
fn rail_form(rail: &Curve, tolerance: f64) -> Result<WireForm, GeomError> {
    wire_form(rail, tolerance)
}

/// The start point and unit tangent of a rail (where a pipe's section
/// sits and the direction it faces).
fn rail_start(form: &WireForm) -> Result<(Point, DVec3), GeomError> {
    match form {
        WireForm::Chain { vertices, .. } => {
            let (Some(&a), Some(&b)) = (vertices.first(), vertices.get(1)) else {
                return Err(GeomError::DegenerateCurve {
                    reason: "rail has fewer than two distinct vertices".to_owned(),
                });
            };
            Ok((a, (b.0 - a.0).normalize()))
        }
        WireForm::Circle { frame, radius } => Ok((frame.point_at(*radius, 0.0), frame.y)),
    }
}

/// A right-handed frame at `origin` whose z is `normal` (unit); the x axis
/// is the world axis least aligned with the normal, projected — a
/// deterministic choice with no preferred direction in the plane.
#[cfg(feature = "occt")]
fn frame_normal_to(origin: Point, normal: DVec3) -> Frame {
    let candidates = [DVec3::X, DVec3::Y, DVec3::Z];
    let mut best = DVec3::X;
    let mut best_alignment = f64::INFINITY;
    for candidate in candidates {
        let alignment = candidate.dot(normal).abs();
        if alignment < best_alignment {
            best_alignment = alignment;
            best = candidate;
        }
    }
    let x = (best - normal * best.dot(normal)).normalize();
    let y = normal.cross(x);
    Frame {
        origin,
        x,
        y,
        z: normal,
    }
}

/// The revolution axis as (origin, unit direction) from a `Line` curve;
/// anything else is refused.
fn axis_of(axis: &Curve, tolerance: f64) -> Result<(Point, DVec3), GeomError> {
    let Curve::Line(line) = axis else {
        return Err(GeomError::BadParameter {
            name: "axis",
            value: axis.variant_name().to_owned(),
            requirement: "must be a Line (the `line` node)",
        });
    };
    let direction = line.b.0 - line.a.0;
    let length = direction.length();
    if !(length.is_finite() && length > tolerance) {
        return Err(GeomError::DegenerateCurve {
            reason: format!("revolution axis has length {length} (tolerance {tolerance})"),
        });
    }
    Ok((line.a, direction / length))
}

/// A profile may touch the revolution axis but never cross it: every
/// vertex on one side (or on it), the circle not cut by it.
fn require_one_side(
    profile: &Profile,
    origin: Point,
    direction: DVec3,
    tolerance: f64,
) -> Result<(), GeomError> {
    // The axis must lie in the profile plane.
    let off_plane = |point: DVec3| (point - profile.frame.origin.0).dot(profile.frame.z);
    for (what, point) in [("its start", origin.0), ("its end", origin.0 + direction)] {
        let distance = off_plane(point);
        if !tol::near_zero(distance, tolerance) {
            return Err(GeomError::BadParameter {
                name: "axis",
                value: format!("{what} lies {distance} from the profile plane"),
                requirement: "the revolution axis must lie in the profile's plane",
            });
        }
    }
    // In-plane side of each point: sign of (axis direction × offset) · normal.
    let side = |point: DVec3| direction.cross(point - origin.0).dot(profile.frame.z);
    match &profile.form {
        WireForm::Chain { vertices, .. } => {
            let mut positive = false;
            let mut negative = false;
            for vertex in vertices {
                let s = side(vertex.0);
                if s > tolerance {
                    positive = true;
                } else if s < -tolerance {
                    negative = true;
                }
            }
            if positive && negative {
                return Err(GeomError::BadParameter {
                    name: "profile",
                    value: "vertices on both sides of the axis".to_owned(),
                    requirement: "a revolved profile must stay on one side of its axis \
                                  (it may touch it)",
                });
            }
        }
        WireForm::Circle { frame, radius } => {
            let distance = side(frame.origin.0).abs();
            if distance + tolerance < *radius {
                return Err(GeomError::BadParameter {
                    name: "profile",
                    value: format!("circle centre {distance} from the axis, radius {radius}"),
                    requirement: "a revolved circle must stay on one side of its axis",
                });
            }
        }
    }
    Ok(())
}

/// The sweep `angle` domain as (start, sweep) radians: a non-empty sweep
/// of at most a full turn, in either direction.
fn sweep_angle(angle: Domain, tolerance_angle: f64) -> Result<(f64, f64), GeomError> {
    let sweep = angle.end - angle.start;
    if !(sweep.is_finite() && angle.start.is_finite()) || sweep.abs() <= tolerance_angle {
        return Err(GeomError::BadParameter {
            name: "angle",
            value: format!("{}..{}", angle.start, angle.end),
            requirement: "must span a non-empty angle (radians)",
        });
    }
    if sweep.abs() > std::f64::consts::TAU + tolerance_angle {
        return Err(GeomError::BadParameter {
            name: "angle",
            value: format!("{}..{}", angle.start, angle.end),
            requirement: "must span at most a full turn (2π radians)",
        });
    }
    Ok((
        angle.start,
        sweep.clamp(-std::f64::consts::TAU, std::f64::consts::TAU),
    ))
}

#[cfg(feature = "occt")]
mod node_backend {
    use super::{
        Deflection, Profile, VolumeProperties, axis_of, frame_normal_to, rail_form, rail_start,
        require_one_side, sweep_angle,
    };
    use crate::GeomError;
    use crate::curve::WireForm;
    use crate::frame::Frame;
    use crate::occt::{Handle, Wire};
    use crate::transform::Similarity;
    use cicada_core::geometry::{Curve, Solid};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Point, Vector};
    use glam::DVec3;

    fn handles(solids: &[Solid]) -> Result<Vec<Handle>, GeomError> {
        solids.iter().map(Handle::from_value).collect()
    }

    pub fn box_in_frame(frame: &Frame, extents: DVec3) -> Result<Solid, GeomError> {
        Handle::box_in_frame(frame, extents)?.into_value()
    }

    pub fn sphere(frame: &Frame, radius: f64) -> Result<Solid, GeomError> {
        Handle::sphere(frame, radius)?.into_value()
    }

    pub fn cylinder(frame: &Frame, radius: f64, height: f64) -> Result<Solid, GeomError> {
        Handle::cylinder(frame, radius, height)?.into_value()
    }

    pub fn cone(
        frame: &Frame,
        radius1: f64,
        radius2: f64,
        height: f64,
    ) -> Result<Solid, GeomError> {
        Handle::cone(frame, radius1, radius2, height)?.into_value()
    }

    pub fn extrude(profile: &Profile, direction: Vector) -> Result<Solid, GeomError> {
        let wire = Wire::from_form(&profile.form)?;
        Handle::prism(&wire, direction)?.into_value()
    }

    pub fn extrude_to_point(profile: &Profile, apex: Point) -> Result<Solid, GeomError> {
        let wire = Wire::from_form(&profile.form)?;
        Handle::thru_sections(std::slice::from_ref(&wire), true, Some(apex))?.into_value()
    }

    pub fn loft(profiles: &[Profile], ruled: bool) -> Result<Solid, GeomError> {
        let wires = profiles
            .iter()
            .map(|p| Wire::from_form(&p.form))
            .collect::<Result<Vec<_>, _>>()?;
        Handle::thru_sections(&wires, ruled, None)?.into_value()
    }

    pub fn revolve(
        profile: &Profile,
        axis: &Curve,
        angle: Domain,
        tolerance: f64,
        tolerance_angle: f64,
    ) -> Result<Solid, GeomError> {
        let (origin, direction) = axis_of(axis, tolerance)?;
        require_one_side(profile, origin, direction, tolerance)?;
        let (start, sweep) = sweep_angle(angle, tolerance_angle)?;
        let wire = Wire::from_form(&profile.form)?;
        let swept = Handle::revolve(&wire, origin, Vector(direction), sweep.abs())?;
        // A negative sweep turns the other way; `start` rotates the whole
        // result into place. Both are one rigid kernel transform (exact on
        // the analytic surfaces), applied only when needed.
        let mut rotation = 0.0;
        if sweep < 0.0 {
            rotation += sweep; // the solid spans [0, |sweep|]; bring it to [sweep, 0]
        }
        rotation += start;
        if crate::tol::near_zero(rotation, tolerance_angle) {
            return swept.into_value();
        }
        let similarity = Similarity::rotation(&frame_normal_to(origin, direction), rotation);
        swept.transformed(&similarity.coefficients())?.into_value()
    }

    pub fn sweep(rail: &Curve, profile: &Profile, tolerance: f64) -> Result<Solid, GeomError> {
        let spine = Wire::from_form(&rail_form(rail, tolerance)?)?;
        let wire = Wire::from_form(&profile.form)?;
        Handle::sweep(&spine, &wire)?.into_value()
    }

    pub fn pipe(rail: &Curve, radius: f64, tolerance: f64) -> Result<Solid, GeomError> {
        let form = rail_form(rail, tolerance)?;
        let (start, tangent) = rail_start(&form)?;
        let section = WireForm::Circle {
            frame: frame_normal_to(start, tangent),
            radius,
        };
        let spine = Wire::from_form(&form)?;
        let wire = Wire::from_form(&section)?;
        Handle::sweep(&spine, &wire)?.into_value()
    }

    pub fn union_all(solids: &[Solid]) -> Result<Solid, GeomError> {
        let mut all = handles(solids)?;
        let first = all.remove(0);
        if all.is_empty() {
            return first.into_value();
        }
        first.union(all)?.into_value()
    }

    pub fn difference_all(solid: &Solid, cutters: &[Solid]) -> Result<Solid, GeomError> {
        let solid = Handle::from_value(solid)?;
        if cutters.is_empty() {
            return solid.into_value();
        }
        solid.difference_all(handles(cutters)?)?.into_value()
    }

    pub fn intersection(a: &Solid, b: &Solid) -> Result<Solid, GeomError> {
        Handle::from_value(a)?
            .intersection(Handle::from_value(b)?)?
            .into_value()
    }

    pub fn volume(solid: &Solid) -> Result<VolumeProperties, GeomError> {
        Handle::from_value(solid)?.volume()
    }

    pub fn bounds(solid: &Solid) -> Result<(Point, Point), GeomError> {
        Handle::from_value(solid)?.bounds()
    }

    pub fn transform(solid: &Solid, similarity: &Similarity) -> Result<Solid, GeomError> {
        Handle::from_value(solid)?
            .transformed(&similarity.coefficients())?
            .into_value()
    }

    pub fn section(
        solid: &Solid,
        frame: &Frame,
        tolerance: f64,
        deflection: Deflection,
    ) -> Result<Vec<Curve>, GeomError> {
        Handle::from_value(solid)?.section(frame, tolerance, deflection)
    }

    pub fn edges_and_vertices(
        solid: &Solid,
        deflection: Deflection,
    ) -> Result<(Vec<Curve>, Vec<Point>, usize), GeomError> {
        let handle = Handle::from_value(solid)?;
        let (edges, faces) = handle.edges(deflection)?;
        let vertices = handle.vertices()?;
        Ok((edges, vertices, faces))
    }

    pub fn write_step(
        solids: &[Solid],
        path: &str,
        millimeters: f64,
        name: &str,
    ) -> Result<(), GeomError> {
        Handle::write_step(handles(solids)?, path, millimeters, name)
    }

    pub fn read_step(path: &str, millimeters: f64) -> Result<Vec<Solid>, GeomError> {
        Handle::read_step(path, millimeters)?
            .into_iter()
            .map(Handle::into_value)
            .collect()
    }
}

#[cfg(not(feature = "occt"))]
mod node_backend {
    use super::{Deflection, Profile, VolumeProperties};
    use crate::GeomError;
    use crate::frame::Frame;
    use crate::transform::Similarity;
    use cicada_core::geometry::{Curve, Solid};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Point, Vector};
    use glam::DVec3;

    const fn unavailable<T>(operation: &'static str) -> Result<T, GeomError> {
        Err(GeomError::KernelUnavailable {
            kernel: "OCCT",
            feature: "occt",
            operation,
        })
    }

    pub fn box_in_frame(_frame: &Frame, _extents: DVec3) -> Result<Solid, GeomError> {
        unavailable("box")
    }

    pub fn sphere(_frame: &Frame, _radius: f64) -> Result<Solid, GeomError> {
        unavailable("sphere")
    }

    pub fn cylinder(_frame: &Frame, _radius: f64, _height: f64) -> Result<Solid, GeomError> {
        unavailable("cylinder")
    }

    pub fn cone(_frame: &Frame, _r1: f64, _r2: f64, _height: f64) -> Result<Solid, GeomError> {
        unavailable("cone")
    }

    pub fn extrude(_profile: &Profile, _direction: Vector) -> Result<Solid, GeomError> {
        unavailable("extrude")
    }

    pub fn extrude_to_point(_profile: &Profile, _apex: Point) -> Result<Solid, GeomError> {
        unavailable("extrude_to_point")
    }

    pub fn loft(_profiles: &[Profile], _ruled: bool) -> Result<Solid, GeomError> {
        unavailable("loft")
    }

    pub fn revolve(
        _profile: &Profile,
        _axis: &Curve,
        _angle: Domain,
        _tolerance: f64,
        _tolerance_angle: f64,
    ) -> Result<Solid, GeomError> {
        unavailable("revolve")
    }

    pub fn sweep(_rail: &Curve, _profile: &Profile, _tolerance: f64) -> Result<Solid, GeomError> {
        unavailable("sweep")
    }

    pub fn pipe(_rail: &Curve, _radius: f64, _tolerance: f64) -> Result<Solid, GeomError> {
        unavailable("pipe")
    }

    pub fn union_all(_solids: &[Solid]) -> Result<Solid, GeomError> {
        unavailable("solid_union")
    }

    pub fn difference_all(_solid: &Solid, _cutters: &[Solid]) -> Result<Solid, GeomError> {
        unavailable("solid_difference")
    }

    pub fn intersection(_a: &Solid, _b: &Solid) -> Result<Solid, GeomError> {
        unavailable("solid_intersection")
    }

    pub fn volume(_solid: &Solid) -> Result<VolumeProperties, GeomError> {
        unavailable("volume")
    }

    pub fn bounds(_solid: &Solid) -> Result<(Point, Point), GeomError> {
        unavailable("bounding_box")
    }

    pub fn transform(_solid: &Solid, _similarity: &Similarity) -> Result<Solid, GeomError> {
        unavailable("transform")
    }

    pub fn section(
        _solid: &Solid,
        _frame: &Frame,
        _tolerance: f64,
        _deflection: Deflection,
    ) -> Result<Vec<Curve>, GeomError> {
        unavailable("section")
    }

    pub fn edges_and_vertices(
        _solid: &Solid,
        _deflection: Deflection,
    ) -> Result<(Vec<Curve>, Vec<Point>, usize), GeomError> {
        unavailable("deconstruct_solid")
    }

    pub fn write_step(
        _solids: &[Solid],
        _path: &str,
        _millimeters: f64,
        _name: &str,
    ) -> Result<(), GeomError> {
        unavailable("export_step")
    }

    pub fn read_step(_path: &str, _millimeters: f64) -> Result<Vec<Solid>, GeomError> {
        unavailable("import_step")
    }
}

/// A box in a plane's frame spanning the three domains along its axes
/// (decreasing domains normalized; the box's minimum corner is at the
/// domains' starts). `BRepPrimAPI_MakeBox` in the frame — in the world
/// frame the bytes equal [`box_at`]'s.
///
/// # Errors
///
/// [`GeomError::DegenerateFrame`] for a bad plane; [`GeomError::BadParameter`]
/// for an extent empty at `tolerance`; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn box_in_plane(
    plane: &Plane,
    x: Domain,
    y: Domain,
    z: Domain,
    tolerance: f64,
) -> Result<Solid, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    let mut extents = DVec3::ZERO;
    let mut starts = DVec3::ZERO;
    for (index, (name, domain)) in [("x", x), ("y", y), ("z", z)].into_iter().enumerate() {
        let (start, end) = (domain.start.min(domain.end), domain.start.max(domain.end));
        let extent = end - start;
        if !(extent.is_finite() && extent > tolerance) {
            return Err(GeomError::BadParameter {
                name,
                value: format!("{}..{}", domain.start, domain.end),
                requirement: "box extent must be above tolerance",
            });
        }
        extents[index] = extent;
        starts[index] = start;
    }
    let origin = frame.point_at_3(starts.x, starts.y, starts.z);
    let frame = Frame { origin, ..frame };
    node_backend::box_in_frame(&frame, extents)
}

/// A sphere centred at the plane's origin.
///
/// # Errors
///
/// [`GeomError::DegenerateFrame`] for a bad plane; [`GeomError::BadParameter`]
/// for a radius not above `tolerance`; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn sphere(plane: &Plane, radius: f64, tolerance: f64) -> Result<Solid, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    above("radius", radius, tolerance)?;
    node_backend::sphere(&frame, radius)
}

/// A cylinder standing on the plane, `height` along its normal.
///
/// # Errors
///
/// As [`sphere`], plus a height not above `tolerance`.
pub fn cylinder(
    plane: &Plane,
    radius: f64,
    height: f64,
    tolerance: f64,
) -> Result<Solid, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    above("radius", radius, tolerance)?;
    above("height", height, tolerance)?;
    node_backend::cylinder(&frame, radius, height)
}

/// A cone standing on the plane: `radius` at the base, the apex `height`
/// along the normal.
///
/// # Errors
///
/// As [`cylinder`].
pub fn cone(plane: &Plane, radius: f64, height: f64, tolerance: f64) -> Result<Solid, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    above("radius", radius, tolerance)?;
    above("height", height, tolerance)?;
    node_backend::cone(&frame, radius, 0.0, height)
}

/// A closed planar profile extruded along `direction` — exact edges for
/// every curve kind (a circle becomes a cylinder, a polyline a prism).
///
/// # Errors
///
/// [`Profile::closed`]'s refusals; [`GeomError::BadParameter`] for a
/// direction in the profile plane; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn extrude(profile: &Curve, direction: Vector, tolerance: f64) -> Result<Solid, GeomError> {
    let profile = Profile::closed(profile, tolerance)?;
    profile.require_leaving(direction, tolerance)?;
    node_backend::extrude(&profile, direction)
}

/// A closed planar profile tapered to `apex` (a pyramid over a polygon, a
/// cone over a circle).
///
/// # Errors
///
/// [`Profile::closed`]'s refusals; [`GeomError::BadParameter`] for an apex
/// in the profile plane; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn extrude_to_point(profile: &Curve, apex: Point, tolerance: f64) -> Result<Solid, GeomError> {
    let profile = Profile::closed(profile, tolerance)?;
    let height = (apex.0 - profile.frame.origin.0).dot(profile.frame.z);
    if !apex.0.is_finite() || tol::near_zero(height, tolerance) {
        return Err(GeomError::BadParameter {
            name: "apex",
            value: format!("{:?}", apex.0),
            requirement: "must be finite and off the profile plane",
        });
    }
    node_backend::extrude_to_point(&profile, apex)
}

/// A solid through two or more closed profiles in order, ruled (straight
/// between consecutive profiles) or smooth.
///
/// # Errors
///
/// [`GeomError::BadParameter`] for fewer than two profiles;
/// [`Profile::closed`]'s refusals (the message names the profile's index);
/// the kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn loft(profiles: &[Curve], ruled: bool, tolerance: f64) -> Result<Solid, GeomError> {
    if profiles.len() < 2 {
        return Err(GeomError::BadParameter {
            name: "profiles",
            value: profiles.len().to_string(),
            requirement: "a loft needs at least two profiles",
        });
    }
    let profiles = profiles
        .iter()
        .enumerate()
        .map(|(index, curve)| {
            Profile::closed(curve, tolerance).map_err(|error| GeomError::DegenerateCurve {
                reason: format!("profile {index}: {error}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    node_backend::loft(&profiles, ruled)
}

/// A closed planar profile revolved about `axis` (a `Line` in the
/// profile's plane, not crossing the profile) through the `angle` domain
/// (radians; at most a full turn).
///
/// # Errors
///
/// [`Profile::closed`]'s refusals; [`GeomError::BadParameter`] for a
/// non-line axis, an axis off the profile plane or crossing it, or an empty
/// / over-full angle; the kernel's errors; [`GeomError::KernelUnavailable`]
/// without `occt`.
pub fn revolve(
    profile: &Curve,
    axis: &Curve,
    angle: Domain,
    tolerance: f64,
    tolerance_angle: f64,
) -> Result<Solid, GeomError> {
    let profile = Profile::closed(profile, tolerance)?;
    // Validate before the backend so a kernel-free build reports the input
    // errors it can see, and `KernelUnavailable` only for valid input.
    let (origin, direction) = axis_of(axis, tolerance)?;
    require_one_side(&profile, origin, direction, tolerance)?;
    sweep_angle(angle, tolerance_angle)?;
    node_backend::revolve(&profile, axis, angle, tolerance, tolerance_angle)
}

/// A closed profile swept along a rail (GH Sweep1): the profile keeps its
/// orientation relative to the rail's tangent; corners are mitred.
///
/// # Errors
///
/// [`Profile::closed`]'s refusals; the rail's degenerate-curve refusals;
/// the kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn sweep(rail: &Curve, profile: &Curve, tolerance: f64) -> Result<Solid, GeomError> {
    let profile = Profile::closed(profile, tolerance)?;
    rail_form(rail, tolerance)?;
    node_backend::sweep(rail, &profile, tolerance)
}

/// A circle of `radius` swept along a rail, the section perpendicular to
/// the rail at its start.
///
/// # Errors
///
/// The rail's degenerate-curve refusals; [`GeomError::BadParameter`] for a
/// radius not above `tolerance`; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn pipe(rail: &Curve, radius: f64, tolerance: f64) -> Result<Solid, GeomError> {
    above("radius", radius, tolerance)?;
    rail_start(&rail_form(rail, tolerance)?)?;
    node_backend::pipe(rail, radius, tolerance)
}

/// The union of one or more solids (one general-fuse pass, coplanar faces
/// merged). A single solid passes through unchanged (re-serialized).
///
/// # Errors
///
/// [`GeomError::BadParameter`] for an empty list; [`GeomError::Kernel`]
/// when the fuse fails or the operands are disjoint (several bodies);
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn union_all(solids: &[Solid]) -> Result<Solid, GeomError> {
    if solids.is_empty() {
        return Err(GeomError::BadParameter {
            name: "solids",
            value: "[]".to_owned(),
            requirement: "a union needs at least one solid",
        });
    }
    node_backend::union_all(solids)
}

/// `solid` minus every cutter (one pass, coplanar faces merged). No
/// cutters: the solid, re-serialized.
///
/// # Errors
///
/// [`GeomError::Kernel`] when the cut fails, splits the solid or empties
/// it; [`GeomError::KernelUnavailable`] without `occt`.
pub fn difference_all(solid: &Solid, cutters: &[Solid]) -> Result<Solid, GeomError> {
    node_backend::difference_all(solid, cutters)
}

/// The common volume of two solids.
///
/// # Errors
///
/// [`GeomError::Kernel`] when the intersection fails, is empty or is
/// several bodies; [`GeomError::KernelUnavailable`] without `occt`.
pub fn intersection(a: &Solid, b: &Solid) -> Result<Solid, GeomError> {
    node_backend::intersection(a, b)
}

/// Volume and centroid.
///
/// # Errors
///
/// The kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn volume(solid: &Solid) -> Result<VolumeProperties, GeomError> {
    node_backend::volume(solid)
}

/// The tight world-aligned bounds (min corner, max corner).
///
/// # Errors
///
/// The kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn bounds(solid: &Solid) -> Result<(Point, Point), GeomError> {
    node_backend::bounds(solid)
}

/// The solid under a similarity (rigid motion × uniform scale, reflections
/// included): the kernel rewrites the geometry, so a moved solid's bytes
/// describe the moved geometry.
///
/// # Errors
///
/// The kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn transform(solid: &Solid, similarity: &Similarity) -> Result<Solid, GeomError> {
    node_backend::transform(solid, similarity)
}

/// The planar section of a solid through `plane`: one closed curve per
/// loop (circles exact, the rest polylines at `deflection`); empty when
/// the plane misses the solid.
///
/// # Errors
///
/// [`GeomError::DegenerateFrame`] for a bad plane; the kernel's errors;
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn section(
    solid: &Solid,
    plane: &Plane,
    tolerance: f64,
    deflection: Deflection,
) -> Result<Vec<Curve>, GeomError> {
    let frame = orthonormal(plane, tolerance)?;
    node_backend::section(solid, &frame, tolerance, deflection)
}

/// A solid's distinct edges (lines and full circles exact, the rest
/// polylines at `deflection`), distinct vertices, and face count.
///
/// # Errors
///
/// The kernel's errors; [`GeomError::KernelUnavailable`] without `occt`.
pub fn edges_and_vertices(
    solid: &Solid,
    deflection: Deflection,
) -> Result<(Vec<Curve>, Vec<Point>, usize), GeomError> {
    node_backend::edges_and_vertices(solid, deflection)
}

/// Write solids to a STEP AP214 file, byte-deterministic for the same
/// solids (fixed header, [`STEP_TIMESTAMP`]); `millimeters` is
/// the document unit's size, declared in the file; `name` is the header's
/// product/file name.
///
/// # Errors
///
/// [`GeomError::BadParameter`] for an empty list; the kernel's errors
/// (including an unwritable path); [`GeomError::KernelUnavailable`] without
/// `occt`.
pub fn write_step(
    solids: &[Solid],
    path: &str,
    millimeters: f64,
    name: &str,
) -> Result<(), GeomError> {
    node_backend::write_step(solids, path, millimeters, name)
}

/// Every solid of a STEP file, scaled into a document whose unit is
/// `millimeters` long.
///
/// # Errors
///
/// The kernel's errors (an unreadable file, no solids);
/// [`GeomError::KernelUnavailable`] without `occt`.
pub fn read_step(path: &str, millimeters: f64) -> Result<Vec<Solid>, GeomError> {
    node_backend::read_step(path, millimeters)
}

fn above(name: &'static str, value: f64, tolerance: f64) -> Result<(), GeomError> {
    if value.is_finite() && value > tolerance {
        Ok(())
    } else {
        Err(GeomError::BadParameter {
            name,
            value: format!("{value}"),
            requirement: "must be finite and above tolerance",
        })
    }
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
    fn deflection_refuses_values_below_the_kernel_floor() {
        // Positive, finite, and still refused: the mesher throws for a
        // linear deflection under Precision::Confusion (1e-7) and an
        // angular one under Precision::Angular (1e-12). The refusal is
        // ours, typed, and names the floor — the kernel never sees it.
        let error = Deflection::new(1e-12, 0.1).expect_err("below the linear floor");
        match &error {
            GeomError::BadParameter {
                name: "linear_deflection",
                value,
                requirement,
            } => {
                assert_eq!(value, "0.000000000001");
                assert!(requirement.contains("1e-7"), "{requirement}");
                assert!(
                    requirement.contains("Precision::Confusion"),
                    "{requirement}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            Deflection::new(MIN_LINEAR_DEFLECTION / 2.0, 0.1),
            Err(GeomError::BadParameter {
                name: "linear_deflection",
                ..
            })
        ));
        assert!(matches!(
            Deflection::new(0.1, MIN_ANGULAR_DEFLECTION / 2.0),
            Err(GeomError::BadParameter {
                name: "angular_deflection",
                ..
            })
        ));
        // Exactly the floor is admitted (the kernel's comparison is `<`).
        let floor =
            Deflection::new(MIN_LINEAR_DEFLECTION, MIN_ANGULAR_DEFLECTION).expect("the floor");
        assert!(crate::tol::close(floor.linear(), 1e-7, 1e-22));
        assert!(crate::tol::close(floor.angular(), 1e-12, 1e-27));
    }

    #[test]
    fn display_deflection_is_above_the_floor_for_every_unit() {
        // `Deflection::display` is infallible because the formula cannot
        // reach the floor: the largest unit gives the finest linear
        // deflection (0.02 mm in a foot document is 6.6e-5), still more
        // than two decades above it. Every unit, at the finest tolerances
        // `ProjectConfig` accepts.
        for unit in [
            Unit::Millimeter,
            Unit::Centimeter,
            Unit::Meter,
            Unit::Inch,
            Unit::Foot,
        ] {
            let config = ProjectConfig::new(unit, 1e-12, 1e-12).expect("finest tolerances");
            let display = Deflection::display(&config);
            assert!(
                display.linear() >= MIN_LINEAR_DEFLECTION * 100.0,
                "{unit:?}: linear {} is too close to the floor",
                display.linear()
            );
            assert!(display.angular() >= MIN_ANGULAR_DEFLECTION);
            // And what `display` built, `new` would have accepted.
            assert_eq!(
                Deflection::new(display.linear(), display.angular()).expect("admitted"),
                display
            );
        }
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
