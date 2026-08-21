//! The OCCT seam (docs/03 §The OCCT seam as built; DECISIONS.md rows 16 and
//! 42; built from the probe docs/probes/occt-2026-08.md). Behind the `occt`
//! Cargo feature: a typed, `Result`-returning surface over Ben's fork of
//! `opencascade-rs`, linked against a PREBUILT OCCT 7.8.1 that
//! `tools/fetch_occt.py` installs and `DEP_OCCT_ROOT` names.
//!
//! This module is the KERNEL level: [`Handle`]s are live `TopoDS_Shape`s.
//! The VALUE level — `core::Solid` in, `core::Solid` out, feature-independent
//! signatures — is [`crate::solid`], which every caller outside this crate's
//! tests and benches uses.
//!
//! # The sharing model (v0.1 item 3 WP-B)
//!
//! A `core::Solid` is its canonical bytes; a kernel handle is a derived,
//! op-local artifact. Three rules make two solids on two rayon workers safe
//! without any lock, and make a warm solve compute the same bytes as a cold
//! one:
//!
//! 1. **Every handle exclusively owns its `TShape` graph.** A handle is
//!    born either from a constructor (`BRepPrimAPI_MakeBox`, a prism over a
//!    profile that dies inside the glue call) or from reading canonical
//!    bytes (`BinTools` builds a fresh object graph — the deep copy the
//!    WP-A plan spelled `BRepBuilderAPI_Copy`, obtained for free from the
//!    path every value already takes). No two live handles ever share a
//!    `TShape`.
//! 2. **Kernel operations CONSUME their handles** (`self` by value):
//!    [`Handle::difference`] takes both operands and returns the result,
//!    which shares untouched faces with inputs that no longer exist — so
//!    the result exclusively owns its graph again (rule 1 is an
//!    invariant, not a convention). [`Handle::tessellate`] consumes too:
//!    OCCT attaches the triangulation to the `TShape`s and a later mesh at
//!    a coarser deflection would keep the finer one (`BRepMesh` reuses a
//!    triangulation that already satisfies the request), so a re-used
//!    handle would make a `tessellate` result depend on what was displayed
//!    before it. Booleans update the tolerances of input sub-shapes in
//!    place when the intersection needs it (`BOPAlgo_Builder`'s default,
//!    non-`NonDestructive` mode), so a re-used input would make the NEXT
//!    operation depend on the previous one. Consumption ends both hazards
//!    by type: there is no handle left to re-use.
//! 3. **Results go back to bytes** ([`Handle::into_value`]) and the handle
//!    dies. The next node reads the bytes again. That re-read is the price
//!    of the model; [`crate::solid`]'s docs and docs/03 carry the measured
//!    numbers (a `BinTools` read is a small fraction of the boolean it
//!    feeds).
//!
//! Consequently there is no process-wide kernel lock any more (WP-A's
//! stand-in while the model was undecided): the glue's calls touch no OCCT
//! global — `BRepPrimAPI_*`, `BRepBuilderAPI_*`, `BRepAlgoAPI_Cut` (run
//! sequentially, `RunParallel` off), `BRepMesh_IncrementalMesh`
//! (`isInParallel` off), `BinTools_ShapeSet` and `TopExp_Explorer` keep
//! their state in locals; `Standard_Type` registration and the memory
//! manager are thread-safe in OCCT 7.x; the statics they read
//! (`BRepLib::Precision`, `BOPAlgo_Options::GetParallelMode`) are never
//! written. The one OCCT subsystem known to keep mutable globals is
//! `Interface_Static` (STEP reader/writer parameters): WP-C's
//! `import_step`/`export_step` must run those calls under a lock of their
//! own, documented there. The thread-safety test below runs N rayon
//! workers over M related values (a base, its cutters, their differences)
//! and checks every result against single-threaded goldens.
//!
//! A handle-RE-USE cache (keep handles keyed by value hash across nodes)
//! is deliberately NOT here: under the semantics above a cached handle is
//! pristine only until its first kernel operation, and the measured re-read
//! cost does not justify a cache that would have to evict on every use.
//! [`Handle::from_value`] is the one choke point such a cache would wrap if
//! WP-C's glue makes booleans non-destructive and meshing idempotent.
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
//! triangulation) are turned into errors on the C++ side, and a final
//! `catch (...)` makes the boundary total rather than an inventory of
//! known exception types. Nothing in this module calls a bridge function
//! that lacks `Result` except `TopExp_Explorer`'s `More`/`Next`, which do
//! not throw when `Next` follows a true `More` (the only way it is called
//! here); the tests drive a real `Standard_DomainError` and, through the
//! fork's `cicada_selftest_throw`, one exception of each kind through the
//! boundary to prove every clause is active in this build.
//!
//! # Canonical bytes
//!
//! [`Handle::canonical_bytes`] is OCCT's `BinTools` format at the PINNED
//! [`CANONICAL_FORMAT_VERSION`], written with `theWithTriangles = false,
//! theWithNormals = false`, after normalizing what is history rather than
//! geometry: single-solid compounds are unwrapped at construction, and the
//! per-shape `Free` / `Modified` / `Checked` flags (which `BinTools` writes
//! and which display tessellation flips) are written in a canonical state
//! on a snapshot that is restored afterwards. The bytes are byte-stable
//! across processes and two independent OCCT builds (probe Q2), a fixed
//! point under read → write, and unaffected by tessellating the solid.
//! Cross-OS identity is what the CI `occt` jobs measure.

use std::collections::HashMap;
use std::fmt;

use cicada_core::geometry::{Mesh, SOLID_CANONICAL_HEADER, Solid, Watertight};
use cicada_core::spatial::{Point, Vector};
use cxx::UniquePtr;
use glam::DVec2;
use opencascade_sys::top_abs::TopAbs_ShapeEnum;
use opencascade_sys::top_exp::TopExp_Explorer_new;
use opencascade_sys::topo_ds::TopoDS_Shape;
use opencascade_sys::{bin_tools, cicada as glue};

use crate::frame::polygon_frame;
use crate::solid::{Deflection, Tessellation};
use crate::triangulate::ear_clip;
use crate::{GeomError, tol};

/// The `BinTools_FormatVersion` of [`Handle::canonical_bytes`]: 4 ("Open
/// CASCADE Topology V4", OCCT 7.6+). PINNED — OCCT's `_CURRENT` moves with
/// releases and would silently change every `Solid` hash. Changing this
/// value is a determinism-policy change (DECISIONS.md), not a tweak.
pub const CANONICAL_FORMAT_VERSION: i32 = 4;

/// The first bytes of every canonical serialization (the V4 header) — the
/// same constant core checks at value construction.
pub const CANONICAL_HEADER: &[u8] = SOLID_CANONICAL_HEADER;

/// One live OCCT solid: a `TopoDS_Shape` that IS a single `TopAbs_SOLID`
/// (compounds holding exactly one solid are unwrapped at construction;
/// anything else is refused), exclusively owning its `TShape` graph
/// (module docs §The sharing model). Op-local: it is made from a value or
/// a constructor, used by ONE kernel operation, and either consumed by it
/// or turned back into a value. `Send` (the fork marks `TopoDS_Shape` so)
/// and not `Sync` — a handle belongs to one thread at a time, and nothing
/// else points at its `TShape`s.
pub struct Handle {
    inner: UniquePtr<TopoDS_Shape>,
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("occt::Handle").finish_non_exhaustive()
    }
}

/// A kernel failure attributed to the operation that hit it.
fn kernel(operation: &str, error: &cxx::Exception) -> GeomError {
    GeomError::Kernel {
        reason: format!("OCCT {operation}: {}", error.what()),
    }
}

impl Handle {
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

    /// `self` minus `cutter` (`BRepAlgoAPI_Cut`), consuming both: the
    /// result shares the faces the cut did not touch with its inputs, and
    /// the cut may have raised tolerances inside them — so the inputs are
    /// gone, and the result owns its graph alone (module docs, rule 2).
    /// The result must again be exactly one solid: a cut that splits the
    /// solid in two, or removes it entirely, is refused loudly rather than
    /// returned as a compound.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] with OCCT's error report, or when the result
    /// is not a single solid.
    // Taking `cutter` by value IS the point (rule 2: the cut may raise the
    // tolerances of sub-shapes inside it and the result shares faces with
    // it), so the lint that asks for `&Self` is overruled here on purpose.
    #[allow(clippy::needless_pass_by_value)]
    pub fn difference(self, cutter: Self) -> Result<Self, GeomError> {
        let shape =
            glue::cicada_cut(&self.inner, &cutter.inner).map_err(|error| kernel("cut", &error))?;
        drop(cutter);
        drop(self);
        Self::from_shape(&shape, "cut")
    }

    /// Number of faces (`TopAbs_FACE` sub-shapes, each counted once per
    /// location, as `TopExp_Explorer` visits them).
    #[must_use]
    pub fn face_count(&self) -> usize {
        let mut explorer = TopExp_Explorer_new(&self.inner, TopAbs_ShapeEnum::TopAbs_FACE);
        let mut count = 0;
        while explorer.More() {
            count += 1;
            explorer.pin_mut().Next();
        }
        count
    }

    /// Tessellate (`BRepMesh_IncrementalMesh` at ABSOLUTE linear and
    /// angular deflection) into a welded, watertight core mesh, consuming
    /// the handle (module docs, rule 2: the mesher attaches its
    /// triangulation to the `TShape`s). Per-face vertices are merged
    /// exactly (bit-identical positions, `-0.0` canonicalized to `0.0`
    /// first), zero-area triangles the weld produces are dropped, and the
    /// structural watertight predicate is checked — a tessellation that is
    /// not closed is an error, never a leaky mesh. The face count rides
    /// along so the display path reconstructs once for both.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the mesher fails; [`GeomError::NotWatertight`]
    /// if the welded result is not closed. (A non-positive deflection cannot
    /// reach here: [`Deflection`] is validated at construction.)
    pub fn tessellate(self, deflection: Deflection) -> Result<Tessellation, GeomError> {
        let faces = self.face_count();
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        glue::cicada_tessellate(
            &self.inner,
            deflection.linear(),
            deflection.angular(),
            &mut positions,
            &mut indices,
        )
        .map_err(|error| kernel("tessellate", &error))?;
        drop(self);
        // Welding is pure Rust over our own buffers.
        let mesh = weld(&positions, &indices)?;
        Ok(Tessellation { mesh, faces })
    }

    /// The canonical serialization — see the module docs. Side-effect free
    /// (the flag snapshot is restored), so it does not consume the handle.
    /// Read back with [`Handle::from_canonical_bytes`].
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

    /// Rebuild a solid from [`Handle::canonical_bytes`] output (or any
    /// `BinTools` V1–V4 stream that holds exactly one solid). `BinTools`
    /// builds a fresh object graph: the handle shares nothing with anyone.
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

    /// The handle for a value: its canonical bytes read into a fresh graph.
    /// THE choke point between values and the kernel — every operation in
    /// [`crate::solid`] starts here, and a handle cache (if WP-C's glue ever
    /// makes one sound) would wrap exactly this function.
    ///
    /// # Errors
    ///
    /// As [`Handle::from_canonical_bytes`].
    pub fn from_value(solid: &Solid) -> Result<Self, GeomError> {
        Self::from_canonical_bytes(solid.bytes())
    }

    /// The value of this handle: its canonical bytes, sealed as a
    /// `core::Solid`. Consumes the handle — after this the bytes are the
    /// solid (module docs, rule 3).
    ///
    /// # Errors
    ///
    /// As [`Handle::canonical_bytes`].
    pub fn into_value(self) -> Result<Solid, GeomError> {
        let bytes = self.canonical_bytes()?;
        Solid::from_canonical_bytes(bytes).map_err(|error| GeomError::Serialization {
            reason: error.to_string(),
        })
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
