//! The OCCT seam (docs/03 §The seam; DECISIONS.md rented-kernels and
//! B-rep-first rows; built from the probe docs/probes/occt-2026-08.md).
//! Behind the `occt` Cargo feature: a typed, `Result`-returning surface over
//! Ben's fork of `opencascade-rs`, linked against a PREBUILT OCCT 7.8.1 that
//! `tools/fetch_occt.py` installs and `DEP_OCCT_ROOT` names.
//!
//! What is wrapped (docs/17 Item 3 WP-A's set): a box, a planar-polygon
//! extrusion, the boolean difference, tessellation into core's
//! [`Watertight<Mesh>`], and the canonical byte form a content-hashed
//! `Solid` value (WP-B) needs — in both directions.
//!
//! # Exception policy
//!
//! Every kernel call goes through the fork's `cicada` glue, whose bridge
//! functions are declared `Result` and whose translation units carry a
//! `rust::behavior::trycatch` that catches OCCT's `Standard_Failure` (it
//! does NOT derive from `std::exception`, so cxx's default handler would
//! let it unwind into Rust and abort the process — measured in the probe:
//! exit `0xC0000409`). Failures OCCT reports by status rather than by
//! throwing (boolean error reports, an unfinished mesher, a face without
//! triangulation) are turned into errors on the C++ side. Nothing in this
//! module calls a bridge function that lacks `Result`, so no OCCT exception
//! can reach Rust unhandled; the tests drive a real `Standard_DomainError`
//! through the boundary to prove the hook is active in this build.
//!
//! # Canonical bytes
//!
//! [`Solid::canonical_bytes`] is OCCT's `BinTools` format at the PINNED
//! [`CANONICAL_FORMAT_VERSION`], written with `theWithTriangles = false,
//! theWithNormals = false`, after normalizing what is history rather than
//! geometry: single-solid compounds are unwrapped at construction, and the
//! per-shape `Free` / `Modified` / `Checked` flags (which `BinTools` writes
//! and which display tessellation flips) are written in a canonical state
//! on a snapshot that is restored afterwards. The bytes are byte-stable
//! across processes and two independent OCCT builds (probe Q2), a fixed
//! point under read → write, and unaffected by tessellating the solid.
//! Cross-OS identity is what the CI `occt` jobs measure.
//!
//! # Threads
//!
//! A [`Solid`] is `Send` (the fork marks `TopoDS_Shape` so) and not `Sync`:
//! OCCT attaches tessellation to the shared `TShape`s, so two threads
//! tessellating the same solid would race. WP-B decides the sharing model.

use std::collections::HashMap;
use std::fmt;

use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::spatial::{Point, Vector};
use cxx::UniquePtr;
use glam::DVec2;
use opencascade_sys::topo_ds::TopoDS_Shape;
use opencascade_sys::{bin_tools, cicada as glue};

use crate::frame::polygon_frame;
use crate::triangulate::ear_clip;
use crate::{GeomError, tol};

/// The `BinTools_FormatVersion` of [`Solid::canonical_bytes`]: 4 ("Open
/// CASCADE Topology V4", OCCT 7.6+). PINNED — OCCT's `_CURRENT` moves with
/// releases and would silently change every `Solid` hash. Changing this
/// value is a determinism-policy change (DECISIONS.md), not a tweak.
pub const CANONICAL_FORMAT_VERSION: i32 = 4;

/// The first bytes of every canonical serialization (the V4 header).
pub const CANONICAL_HEADER: &[u8] = b"\nOpen CASCADE Topology V4";

/// One OCCT solid: a `TopoDS_Shape` that IS a single `TopAbs_SOLID`
/// (compounds holding exactly one solid are unwrapped at construction;
/// anything else is refused). Immutable from Rust's point of view.
pub struct Solid {
    inner: UniquePtr<TopoDS_Shape>,
}

impl fmt::Debug for Solid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("occt::Solid").finish_non_exhaustive()
    }
}

/// A kernel failure attributed to the operation that hit it.
fn kernel(operation: &str, error: &cxx::Exception) -> GeomError {
    GeomError::Kernel {
        reason: format!("OCCT {operation}: {}", error.what()),
    }
}

impl Solid {
    /// Wrap a kernel result, unwrapping a single-solid compound and refusing
    /// everything that is not exactly one solid.
    fn from_shape(shape: &TopoDS_Shape, operation: &str) -> Result<Self, GeomError> {
        let inner = glue::cicada_single_solid(shape).map_err(|error| kernel(operation, &error))?;
        Ok(Self { inner })
    }

    /// An axis-aligned box with its minimum corner at `min_corner` and
    /// positive `extents` along x, y, z (`BRepPrimAPI_MakeBox`).
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-finite or non-positive extent;
    /// [`GeomError::Kernel`] if OCCT refuses.
    pub fn box_at(min_corner: Point, extents: Vector) -> Result<Self, GeomError> {
        for (name, value) in [
            ("dx", extents.0.x),
            ("dy", extents.0.y),
            ("dz", extents.0.z),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(GeomError::BadParameter {
                    name,
                    value: format!("{value}"),
                    requirement: "box extents must be finite and > 0",
                });
            }
        }
        if !min_corner.0.is_finite() {
            return Err(GeomError::BadParameter {
                name: "min_corner",
                value: format!("{:?}", min_corner.0),
                requirement: "must be finite",
            });
        }
        let shape = glue::cicada_make_box(
            min_corner.0.x,
            min_corner.0.y,
            min_corner.0.z,
            extents.0.x,
            extents.0.y,
            extents.0.z,
        )
        .map_err(|error| kernel("make_box", &error))?;
        Self::from_shape(&shape, "make_box")
    }

    /// A closed planar polygon (vertices in order; the closing edge is
    /// implied) extruded along `direction` into a prism
    /// (`BRepBuilderAPI_MakeEdge/Wire/Face` + `BRepPrimAPI_MakePrism`).
    ///
    /// The profile is validated HERE, with the mesh tier's rules and the
    /// explicit `tolerance`: at least three vertices, planar within
    /// `tolerance`, simple (ear-clipping succeeds, which refuses
    /// zero-area and self-intersecting loops), and a direction that leaves
    /// the plane. OCCT itself accepts collinear points and returns a
    /// zero-volume solid (measured), so the kernel is not the validator.
    ///
    /// # Errors
    ///
    /// [`GeomError::NotPlanar`], [`GeomError::NotSimple`],
    /// [`GeomError::DegenerateFrame`], [`GeomError::BadParameter`] for the
    /// profile/direction; [`GeomError::Kernel`] if OCCT refuses.
    pub fn extrude_polygon(
        profile: &[Point],
        direction: Vector,
        tolerance: f64,
    ) -> Result<Self, GeomError> {
        if profile.len() < 3 {
            return Err(GeomError::NotSimple {
                reason: format!("{} vertices (need 3)", profile.len()),
            });
        }
        let frame = polygon_frame(profile, tolerance)?;
        let mut flat = Vec::with_capacity(profile.len());
        for (vertex, point) in profile.iter().enumerate() {
            let local = frame.coordinates(*point);
            if !tol::near_zero(local.z, tolerance) {
                return Err(GeomError::NotPlanar {
                    vertex,
                    distance: local.z,
                });
            }
            flat.push(DVec2::new(local.x, local.y));
        }
        // Simplicity + non-zero area, with the same refusals the mesh tier
        // gives; the triangles themselves are not needed.
        ear_clip(&flat, tolerance)?;
        if !direction.0.is_finite() || tol::near_zero(direction.0.dot(frame.z), tolerance) {
            return Err(GeomError::BadParameter {
                name: "direction",
                value: format!("{:?}", direction.0),
                requirement: "must be finite and leave the profile plane (not be parallel to it)",
            });
        }
        let mut xyz = Vec::with_capacity(profile.len() * 3);
        for point in profile {
            xyz.extend_from_slice(&[point.0.x, point.0.y, point.0.z]);
        }
        let shape = glue::cicada_extrude_polygon(&xyz, direction.0.x, direction.0.y, direction.0.z)
            .map_err(|error| kernel("extrude_polygon", &error))?;
        Self::from_shape(&shape, "extrude_polygon")
    }

    /// `self` minus `cutter` (`BRepAlgoAPI_Cut`). The result must again be
    /// exactly one solid: a cut that splits the solid in two, or removes it
    /// entirely, is refused loudly rather than returned as a compound.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] with OCCT's error report, or when the result
    /// is not a single solid.
    pub fn difference(&self, cutter: &Self) -> Result<Self, GeomError> {
        let shape =
            glue::cicada_cut(&self.inner, &cutter.inner).map_err(|error| kernel("cut", &error))?;
        Self::from_shape(&shape, "cut")
    }

    /// Tessellate (`BRepMesh_IncrementalMesh` at ABSOLUTE `linear_deflection`
    /// and `angular_deflection` in radians) into a welded, watertight core
    /// mesh: per-face vertices are merged exactly (bit-identical positions,
    /// `-0.0` canonicalized to `0.0` first), zero-area triangles the weld
    /// produces are dropped, and the structural watertight predicate is
    /// checked — a tessellation that is not closed is an error, never a
    /// leaky mesh.
    ///
    /// Meshing attaches the triangulation to the shape's shared `TShape`s
    /// (OCCT semantics); [`Solid::canonical_bytes`] is unaffected by design.
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-positive deflection;
    /// [`GeomError::Kernel`] if the mesher fails; [`GeomError::NotWatertight`]
    /// if the welded result is not closed.
    pub fn tessellate(
        &self,
        linear_deflection: f64,
        angular_deflection: f64,
    ) -> Result<Watertight<Mesh>, GeomError> {
        for (name, value) in [
            ("linear_deflection", linear_deflection),
            ("angular_deflection", angular_deflection),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(GeomError::BadParameter {
                    name,
                    value: format!("{value}"),
                    requirement: "must be finite and > 0",
                });
            }
        }
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        glue::cicada_tessellate(
            &self.inner,
            linear_deflection,
            angular_deflection,
            &mut positions,
            &mut indices,
        )
        .map_err(|error| kernel("tessellate", &error))?;
        weld(&positions, &indices)
    }

    /// The canonical serialization — see the module docs. Read back with
    /// [`Solid::from_canonical_bytes`].
    ///
    /// # Errors
    ///
    /// [`GeomError::Serialization`] if the kernel's writer fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GeomError> {
        let bytes = glue::cicada_canonical_bytes(&self.inner, CANONICAL_FORMAT_VERSION).map_err(
            |error| GeomError::Serialization {
                reason: format!("OCCT BinTools write: {}", error.what()),
            },
        )?;
        if !bytes.starts_with(CANONICAL_HEADER) {
            return Err(GeomError::Serialization {
                reason: "OCCT BinTools write produced bytes without the V4 header".to_owned(),
            });
        }
        Ok(bytes)
    }

    /// Rebuild a solid from [`Solid::canonical_bytes`] output (or any
    /// `BinTools` V1–V4 stream that holds exactly one solid).
    ///
    /// # Errors
    ///
    /// [`GeomError::Serialization`] for bytes `BinTools` cannot read;
    /// [`GeomError::Kernel`] if they hold anything but one solid.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, GeomError> {
        let shape =
            bin_tools::read_brep_binary_bytes(bytes).map_err(|error| GeomError::Serialization {
                reason: format!("OCCT BinTools read: {}", error.what()),
            })?;
        Self::from_shape(&shape, "from_canonical_bytes")
    }
}

/// Weld per-face vertices on bit-identical positions, drop the zero-area
/// triangles this can create, and require the result to be watertight.
fn weld(positions: &[f64], indices: &[u32]) -> Result<Watertight<Mesh>, GeomError> {
    let mut remap: HashMap<[u64; 3], u32> = HashMap::with_capacity(positions.len() / 3);
    let mut welded_positions: Vec<f64> = Vec::with_capacity(positions.len());
    let mut vertex_of: Vec<u32> = Vec::with_capacity(positions.len() / 3);
    let (vertices, ragged) = positions.as_chunks::<3>();
    if !ragged.is_empty() {
        return Err(GeomError::Kernel {
            reason: format!(
                "tessellate: position buffer length {} is not a multiple of 3",
                positions.len()
            ),
        });
    }
    let negative_zero = (-0.0f64).to_bits();
    for vertex in vertices {
        if !vertex.iter().all(|c| c.is_finite()) {
            return Err(GeomError::Kernel {
                reason: format!("tessellate: non-finite vertex position {vertex:?}"),
            });
        }
        // -0.0 → 0.0 so the two zeros weld (the value model canonicalizes
        // the same way); a bit test, not a float comparison.
        let canonical = vertex.map(|c| if c.to_bits() == negative_zero { 0.0 } else { c });
        let key = canonical.map(f64::to_bits);
        let next = u32::try_from(welded_positions.len() / 3).map_err(|_| GeomError::Kernel {
            reason: "tessellate: more than u32::MAX welded vertices".to_owned(),
        })?;
        let index = *remap.entry(key).or_insert_with(|| {
            welded_positions.extend_from_slice(&canonical);
            next
        });
        vertex_of.push(index);
    }
    let (triangles, ragged) = indices.as_chunks::<3>();
    if !ragged.is_empty() {
        return Err(GeomError::Kernel {
            reason: format!(
                "tessellate: index buffer length {} is not a multiple of 3",
                indices.len()
            ),
        });
    }
    let mut welded_indices = Vec::with_capacity(indices.len());
    for triangle in triangles {
        let mut mapped = [0u32; 3];
        for (slot, &raw) in mapped.iter_mut().zip(triangle) {
            *slot = *vertex_of
                .get(raw as usize)
                .ok_or_else(|| GeomError::Kernel {
                    reason: format!("tessellate: index {raw} outside the position buffer"),
                })?;
        }
        let [a, b, c] = mapped;
        if a == b || b == c || a == c {
            continue; // zero-area after welding (a shared edge's duplicate)
        }
        welded_indices.extend_from_slice(&mapped);
    }
    let mesh = Mesh::new(welded_positions, welded_indices)?;
    if !mesh.is_watertight() {
        return Err(GeomError::NotWatertight {
            reason: format!(
                "OCCT tessellation is not closed after welding ({} vertices, {} triangles)",
                mesh.vertex_count(),
                mesh.triangle_count()
            ),
        });
    }
    Ok(Watertight(mesh))
}

#[cfg(test)]
mod tests;
