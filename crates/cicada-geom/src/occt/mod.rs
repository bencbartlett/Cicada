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

pub(crate) mod glue;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, Once};

use cicada_core::geometry::{
    Circle, Curve, Line, Mesh, Polyline, SOLID_CANONICAL_HEADER, Solid, Watertight,
};
use cicada_core::spatial::{Plane, Point, Vector};
use cxx::UniquePtr;
use glam::{DVec2, DVec3};
use opencascade_sys::top_abs::TopAbs_ShapeEnum;
use opencascade_sys::top_exp::TopExp_Explorer_new;
use opencascade_sys::topo_ds::TopoDS_Shape;
use opencascade_sys::{bin_tools, cicada as fork};

use crate::curve::WireForm;
use crate::frame::{Frame, polygon_frame};
use crate::solid::{Deflection, DisplayTessellation, Tessellation, VolumeProperties};
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

/// What [`Handle::section`] yields: the closed loops, and how many tangent
/// contacts the plane made with the solid that bound no region and were
/// dropped (a diagnostic count — the tests use it to tell "the plane
/// touched along a line" from "the plane missed").
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// One closed curve per loop.
    pub loops: Vec<Curve>,
    /// Open chains the plane only touched the solid along.
    pub contacts: usize,
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("occt::Handle").finish_non_exhaustive()
    }
}

// The sharing model's type-level belt: `Handle` must NEVER be `Sync`.
// `canonical_bytes(&self)` rewrites and restores `TShape` flags through a
// shared reference, which is sound only while no other thread can hold a
// `&Handle` at the same time. Today that holds because the fork marks
// `TopoDS_Shape` `Send` and not `Sync` (`UniquePtr<T>` inherits both); if a
// future fork revision adds `unsafe impl Sync for TopoDS_Shape`, this block
// stops compiling — the call below becomes ambiguous between the two impls
// — instead of silently reopening the race the model closed.
mod not_sync {
    pub(super) trait NotSync<Marker> {
        fn check() {}
    }
    pub(super) struct IsSync;
    impl<T: ?Sized> NotSync<()> for T {}
    impl<T: ?Sized + Sync> NotSync<IsSync> for T {}
}
const _: fn() = <Handle as not_sync::NotSync<_>>::check;

/// A kernel failure attributed to the operation that hit it. The glue's
/// own messages lead with the bridge function's name (`cicada_cut: …`,
/// `make_box: …`) because one C++ header serves many callers; a node's red
/// text names the OPERATION the user asked for instead, so that prefix is
/// dropped here — a C++ identifier is never the diagnostic.
fn kernel(operation: &str, error: &cxx::Exception) -> GeomError {
    GeomError::Kernel {
        reason: format!("OCCT {operation}: {}", without_glue_prefix(error.what())),
    }
}

/// `what()` without a leading `<glue_function>: ` — an identifier made of
/// lowercase letters, digits and underscores followed by a colon and a
/// space. Anything else is returned unchanged.
fn without_glue_prefix(what: &str) -> &str {
    let Some((head, tail)) = what.split_once(": ") else {
        return what;
    };
    let is_glue_identifier = !head.is_empty()
        && head
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && head.contains('_');
    if is_glue_identifier { tail } else { what }
}

impl Handle {
    /// Wrap a kernel result, unwrapping a single-solid compound and refusing
    /// everything that is not exactly one solid — typed
    /// ([`GeomError::NotOneSolid`] carries the count and the operation), so
    /// a splitting cut or a disjoint union is red with the rule, not with
    /// a glue identifier and a `TopAbs_ShapeEnum` number.
    fn from_shape(shape: &TopoDS_Shape, operation: &str) -> Result<Self, GeomError> {
        let found = usize::try_from(fork::cicada_count_solids(shape)).unwrap_or(0);
        if found != 1 {
            return Err(GeomError::NotOneSolid {
                operation: operation.to_owned(),
                found,
            });
        }
        let inner = fork::cicada_single_solid(shape).map_err(|error| kernel(operation, &error))?;
        Ok(Self { inner })
    }

    /// `BRepCheck_Analyzer`'s verdict on the solid: the kernel's own notion
    /// of a valid B-rep (topology and geometry controls). Diagnostic — the
    /// tests use it to tell an invalid boolean result from a valid one whose
    /// mesh the mesher still cannot close.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the analyzer itself fails.
    pub fn is_valid(&self) -> Result<bool, GeomError> {
        glue::is_valid(&self.inner).map_err(|error| kernel("check", &error))
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
        let shape = fork::cicada_make_box(
            min_corner.0.x,
            min_corner.0.y,
            min_corner.0.z,
            extents.0.x,
            extents.0.y,
            extents.0.z,
        )
        .map_err(|error| kernel("box", &error))?;
        Self::from_shape(&shape, "box")
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
        let shape = fork::cicada_extrude_polygon(&xyz, direction.0.x, direction.0.y, direction.0.z)
            .map_err(|error| kernel("extrude", &error))?;
        Self::from_shape(&shape, "extrude")
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
            fork::cicada_cut(&self.inner, &cutter.inner).map_err(|error| kernel("cut", &error))?;
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
    /// This is the `tessellate` NODE's contract (`Watertight<Mesh>` out:
    /// the mesh tier needs closure). Display does not need it and uses
    /// [`Handle::tessellate_display`], which returns the welded mesh either
    /// way and says whether it closed.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the mesher fails; [`GeomError::NotWatertight`]
    /// if the welded result is not closed. (A non-positive deflection cannot
    /// reach here: [`Deflection`] is validated at construction.)
    pub fn tessellate(self, deflection: Deflection) -> Result<Tessellation, GeomError> {
        closed_or_refused(self.tessellate_display(deflection)?)
    }

    /// The display tessellation: the same mesher and weld as
    /// [`Handle::tessellate`], but closure is REPORTED, not required — a
    /// valid solid whose per-face triangulations do not conform along an
    /// edge (measured: a sphere moved by a kernel transform, minus a
    /// cylinder through both its poles) still draws, with
    /// `watertight == false` for the summary to show. Consumes the handle
    /// (rule 2).
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the mesher fails or hands back malformed
    /// buffers.
    pub fn tessellate_display(
        self,
        deflection: Deflection,
    ) -> Result<DisplayTessellation, GeomError> {
        let faces = self.face_count();
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        fork::cicada_tessellate(
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
        let watertight = mesh.is_watertight();
        Ok(DisplayTessellation {
            mesh,
            watertight,
            faces,
            deflection,
        })
    }

    /// The canonical serialization — see the module docs. Side-effect free
    /// (the flag snapshot is restored), so it does not consume the handle.
    /// Read back with [`Handle::from_canonical_bytes`].
    ///
    /// # Errors
    ///
    /// [`GeomError::Serialization`] if the kernel's writer fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GeomError> {
        let bytes = fork::cicada_canonical_bytes(&self.inner, CANONICAL_FORMAT_VERSION).map_err(
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
        Self::from_shape(&shape, "read")
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

// ---------------------------------------------------------------------------
// The node set (v0.1 item 3 WP-C): cicada-geom's own glue, `glue.hxx`
// ---------------------------------------------------------------------------

/// The STEP translators are the one OCCT subsystem that keeps mutable
/// process-wide state (`Interface_Static`, the work-session controllers'
/// one-time registration, the messenger's printers), so every STEP call —
/// and the one-time quieting of the messenger — runs under this lock
/// (module docs §The sharing model). Nothing else in the seam takes it.
static STEP_LOCK: Mutex<()> = Mutex::new(());

/// Lower OCCT's default printers to failures once per process: the STEP
/// translators narrate at Info level on stdout, and a headless `cicada run`
/// prints its own output and nothing else.
static QUIET: Once = Once::new();

/// The fixed `FILE_NAME.time_stamp` every STEP file carries
/// ([`Handle::write_step`]) — defined at the value level so callers in
/// every build can name it.
pub use crate::solid::STEP_TIMESTAMP;

fn step_guard() -> MutexGuard<'static, ()> {
    // A poisoned lock means a STEP call panicked mid-way on another thread;
    // the state it guards (OCCT statics) is still consistent — the kernel
    // never saw the panic — so continuing is sound.
    STEP_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A frame as the nine doubles the glue reads: origin, unit x, unit z.
fn frame_doubles(frame: &Frame) -> [f64; 9] {
    [
        frame.origin.0.x,
        frame.origin.0.y,
        frame.origin.0.z,
        frame.x.x,
        frame.x.y,
        frame.x.z,
        frame.z.x,
        frame.z.y,
        frame.z.z,
    ]
}

fn xyz_of(points: &[Point]) -> Vec<f64> {
    let mut xyz = Vec::with_capacity(points.len() * 3);
    for point in points {
        xyz.extend_from_slice(&[point.0.x, point.0.y, point.0.z]);
    }
    xyz
}

/// A kernel-level wire (`TopoDS_Wire`): the profile or spine of a sweep,
/// built from a curve's [`WireForm`]. Op-local like [`Handle`]: built for
/// one kernel operation and consumed by it (the prism, loft, revolve and
/// sweep glue read it through a shared reference, and the result shares no
/// `TShape` with a wire that lives on — `BRepBuilderAPI_MakeFace` and the
/// sweeps copy what they keep). Not `Sync`, like every kernel object here.
pub struct Wire {
    inner: UniquePtr<TopoDS_Shape>,
}

impl fmt::Debug for Wire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("occt::Wire").finish_non_exhaustive()
    }
}

impl Wire {
    /// The wire of a curve form: straight edges through a chain's
    /// vertices (closed when the chain is), or one circular edge.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if OCCT refuses (too few distinct points, a
    /// zero radius — both caught earlier by `wire_form`'s own refusals).
    pub fn from_form(form: &WireForm) -> Result<Self, GeomError> {
        let inner = match form {
            WireForm::Chain { vertices, closed } => {
                glue::make_polyline_wire(&xyz_of(vertices), *closed)
                    .map_err(|error| kernel("polyline wire", &error))?
            }
            WireForm::Circle { frame, radius } => {
                glue::make_circle_wire(&frame_doubles(frame), *radius)
                    .map_err(|error| kernel("circle wire", &error))?
            }
        };
        Ok(Self { inner })
    }
}

/// A compound of shapes, consuming the handles it holds: the n-ary
/// booleans' argument lists and the STEP writer's shape list.
fn compound_of<I>(shapes: I) -> Result<UniquePtr<TopoDS_Shape>, GeomError>
where
    I: IntoIterator<Item = UniquePtr<TopoDS_Shape>>,
{
    let mut compound = glue::make_compound().map_err(|error| kernel("compound", &error))?;
    for shape in shapes {
        glue::compound_add(compound.pin_mut(), &shape)
            .map_err(|error| kernel("compound", &error))?;
    }
    Ok(compound)
}

/// Decode the curve records the edge/section glue writes (`kinds`,
/// `counts`, `data` — see glue.hxx): 0 = open polyline (two points → a
/// `Line`), 1 = closed polyline, 2 = a full circle.
fn decode_curves(kinds: &[i32], counts: &[u32], data: &[f64]) -> Result<Vec<Curve>, GeomError> {
    let malformed = |what: &str| GeomError::Kernel {
        reason: format!("curve records from the kernel are malformed: {what}"),
    };
    if kinds.len() != counts.len() {
        return Err(malformed("kinds and counts differ in length"));
    }
    let mut curves = Vec::with_capacity(kinds.len());
    let mut cursor = 0usize;
    let take = |cursor: &mut usize, n: usize| -> Result<&[f64], GeomError> {
        let end = cursor
            .checked_add(n)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| malformed("a record runs past the data buffer"))?;
        let slice = &data[*cursor..end];
        *cursor = end;
        Ok(slice)
    };
    for (&kind, &count) in kinds.iter().zip(counts) {
        match kind {
            0 | 1 => {
                let count = count as usize;
                let slice = take(&mut cursor, count * 3)?;
                let vertices: Vec<Point> = slice
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|&[x, y, z]| Point::new(x, y, z))
                    .collect();
                if vertices.iter().any(|p| !p.0.is_finite()) {
                    return Err(malformed("a non-finite vertex"));
                }
                let closed = kind == 1;
                if (closed && vertices.len() < 3) || (!closed && vertices.len() < 2) {
                    return Err(malformed("a polyline with too few vertices"));
                }
                if !closed && vertices.len() == 2 {
                    curves.push(Curve::Line(Line {
                        a: vertices[0],
                        b: vertices[1],
                    }));
                } else {
                    curves.push(Curve::Polyline(Polyline { vertices, closed }));
                }
            }
            2 => {
                let c = take(&mut cursor, 10)?;
                if c.iter().any(|v| !v.is_finite()) {
                    return Err(malformed("a non-finite circle"));
                }
                curves.push(Curve::Circle(Circle {
                    plane: Plane {
                        origin: Point::new(c[0], c[1], c[2]),
                        x: Vector::new(c[3], c[4], c[5]),
                        y: Vector::new(c[6], c[7], c[8]),
                    },
                    radius: c[9],
                }));
            }
            other => return Err(malformed(&format!("unknown record kind {other}"))),
        }
    }
    if cursor != data.len() {
        return Err(malformed("trailing data after the last record"));
    }
    Ok(curves)
}

impl Handle {
    /// A box in a frame: minimum corner at the frame's origin, positive
    /// extents along its axes (`BRepPrimAPI_MakeBox(gp_Ax2, dx, dy, dz)` —
    /// in the world frame byte-identical to [`Handle::box_at`]).
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-finite or non-positive extent;
    /// [`GeomError::Kernel`] if OCCT refuses.
    pub fn box_in_frame(frame: &Frame, extents: DVec3) -> Result<Self, GeomError> {
        for (name, value) in [("dx", extents.x), ("dy", extents.y), ("dz", extents.z)] {
            if !(value.is_finite() && value > 0.0) {
                return Err(GeomError::BadParameter {
                    name,
                    value: format!("{value}"),
                    requirement: "box extents must be finite and > 0",
                });
            }
        }
        let shape = glue::make_box(&frame_doubles(frame), extents.x, extents.y, extents.z)
            .map_err(|error| kernel("box", &error))?;
        Self::from_shape(&shape, "box")
    }

    /// A sphere centred at the frame's origin (`BRepPrimAPI_MakeSphere`).
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-finite or non-positive radius;
    /// [`GeomError::Kernel`] if OCCT refuses.
    pub fn sphere(frame: &Frame, radius: f64) -> Result<Self, GeomError> {
        positive("radius", radius)?;
        let shape = glue::make_sphere(&frame_doubles(frame), radius)
            .map_err(|error| kernel("sphere", &error))?;
        Self::from_shape(&shape, "sphere")
    }

    /// A cylinder standing on the frame's xy plane, `height` along its z
    /// (`BRepPrimAPI_MakeCylinder`).
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a non-finite or non-positive radius
    /// or height; [`GeomError::Kernel`] if OCCT refuses.
    pub fn cylinder(frame: &Frame, radius: f64, height: f64) -> Result<Self, GeomError> {
        positive("radius", radius)?;
        positive("height", height)?;
        let shape = glue::make_cylinder(&frame_doubles(frame), radius, height)
            .map_err(|error| kernel("cylinder", &error))?;
        Self::from_shape(&shape, "cylinder")
    }

    /// A cone (or frustum) from `radius1` at the frame's xy plane to
    /// `radius2` at `height` along its z (`BRepPrimAPI_MakeCone`); one
    /// radius may be zero (the apex), not both, and they must differ.
    ///
    /// # Errors
    ///
    /// [`GeomError::BadParameter`] for a negative/non-finite radius, a
    /// non-positive height, equal radii or two zero radii;
    /// [`GeomError::Kernel`] if OCCT refuses.
    pub fn cone(frame: &Frame, radius1: f64, radius2: f64, height: f64) -> Result<Self, GeomError> {
        for (name, value) in [("radius", radius1), ("radius2", radius2)] {
            if !(value.is_finite() && value >= 0.0) {
                return Err(GeomError::BadParameter {
                    name,
                    value: format!("{value}"),
                    requirement: "cone radii must be finite and >= 0",
                });
            }
        }
        positive("height", height)?;
        // An exact comparison on purpose: OCCT itself refuses radii within
        // its own resolution of each other, and that refusal arrives as Err.
        #[allow(clippy::float_cmp)]
        if radius1 == radius2 {
            return Err(GeomError::BadParameter {
                name: "radius2",
                value: format!("{radius2}"),
                requirement: "the two radii must differ (equal radii are a cylinder; \
                              both zero is nothing)",
            });
        }
        let shape = glue::make_cone(&frame_doubles(frame), radius1, radius2, height)
            .map_err(|error| kernel("cone", &error))?;
        Self::from_shape(&shape, "cone")
    }

    /// A closed planar wire extruded along `direction` into a prism (the
    /// fork's construction path: over a polyline wire the bytes equal
    /// [`Handle::extrude_polygon`]'s). The caller validated the profile.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the face or the prism cannot be built.
    pub fn prism(profile: &Wire, direction: Vector) -> Result<Self, GeomError> {
        let shape = glue::prism(&profile.inner, direction.0.x, direction.0.y, direction.0.z)
            .map_err(|error| kernel("extrude", &error))?;
        Self::from_shape(&shape, "extrude")
    }

    /// `BRepOffsetAPI_ThruSections` through `sections` in order, as a
    /// solid — ruled (straight between consecutive sections) or smooth —
    /// optionally converging to `apex` after the last section.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the loft cannot be built or is not one
    /// solid.
    pub fn thru_sections(
        sections: &[Wire],
        ruled: bool,
        apex: Option<Point>,
    ) -> Result<Self, GeomError> {
        let mut compound = glue::make_compound().map_err(|error| kernel("compound", &error))?;
        for section in sections {
            glue::compound_add(compound.pin_mut(), &section.inner)
                .map_err(|error| kernel("compound", &error))?;
        }
        let apex: Vec<f64> = apex.map_or_else(Vec::new, |p| vec![p.0.x, p.0.y, p.0.z]);
        let shape =
            glue::thru_sections(&compound, ruled, &apex).map_err(|error| kernel("loft", &error))?;
        Self::from_shape(&shape, "loft")
    }

    /// A closed planar wire revolved by `angle` radians about the axis
    /// through `origin` along `direction` (`BRepPrimAPI_MakeRevol`; a full
    /// turn at 2π). The caller validated the profile against the axis.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if OCCT refuses.
    pub fn revolve(
        profile: &Wire,
        origin: Point,
        direction: Vector,
        angle: f64,
    ) -> Result<Self, GeomError> {
        let axis = [
            origin.0.x,
            origin.0.y,
            origin.0.z,
            direction.0.x,
            direction.0.y,
            direction.0.z,
        ];
        let shape = glue::revolve(&profile.inner, &axis, angle)
            .map_err(|error| kernel("revolve", &error))?;
        Self::from_shape(&shape, "revolve")
    }

    /// A closed wire swept along `spine` into a solid
    /// (`BRepOffsetAPI_MakePipeShell`, corrected Frenet, mitred corners).
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the sweep fails or cannot close.
    pub fn sweep(spine: &Wire, profile: &Wire) -> Result<Self, GeomError> {
        let shape =
            glue::sweep(&spine.inner, &profile.inner).map_err(|error| kernel("sweep", &error))?;
        Self::from_shape(&shape, "sweep")
    }

    /// The union of `self` and `others` in one general-fuse pass, coplanar
    /// faces merged; consumes every operand (module docs, rule 2). Disjoint
    /// operands would fuse into several solids: refused.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] with OCCT's report, or when the result is not
    /// one solid.
    pub fn union(self, others: Vec<Self>) -> Result<Self, GeomError> {
        let arguments = compound_of([self.inner])?;
        let tools = compound_of(others.into_iter().map(|h| h.inner))?;
        let shape = glue::fuse(&arguments, &tools).map_err(|error| kernel("union", &error))?;
        Self::from_shape(&shape, "union")
    }

    /// `self` minus every cutter in one pass, coplanar faces merged;
    /// consumes everything. A cut that splits or empties the solid is
    /// refused, as in [`Handle::difference`].
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] with OCCT's report, or when the result is not
    /// one solid.
    pub fn difference_all(self, cutters: Vec<Self>) -> Result<Self, GeomError> {
        let tools = compound_of(cutters.into_iter().map(|h| h.inner))?;
        let shape = glue::cut(&self.inner, &tools).map_err(|error| kernel("cut", &error))?;
        drop(self);
        Self::from_shape(&shape, "cut")
    }

    /// The common volume of two solids, consuming both; an empty or
    /// multi-body intersection is refused.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] with OCCT's report, or when the result is not
    /// one solid.
    #[allow(clippy::needless_pass_by_value)] // consumption is the sharing model's rule 2
    pub fn intersection(self, other: Self) -> Result<Self, GeomError> {
        let shape = glue::common(&self.inner, &other.inner)
            .map_err(|error| kernel("intersection", &error))?;
        drop(other);
        drop(self);
        Self::from_shape(&shape, "intersection")
    }

    /// Volume and centroid (`BRepGProp::VolumeProperties`, adaptive).
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the integration fails.
    pub fn volume(&self) -> Result<VolumeProperties, GeomError> {
        let mut out = Vec::with_capacity(4);
        glue::volume_properties(&self.inner, 1e-9, &mut out)
            .map_err(|error| kernel("volume", &error))?;
        let [volume, x, y, z] = out[..] else {
            return Err(GeomError::Kernel {
                reason: "volume_properties returned a malformed record".to_owned(),
            });
        };
        if !(volume.is_finite() && x.is_finite() && y.is_finite() && z.is_finite()) {
            return Err(GeomError::Kernel {
                reason: "volume_properties returned a non-finite value".to_owned(),
            });
        }
        Ok(VolumeProperties {
            volume,
            centroid: Point::new(x, y, z),
        })
    }

    /// The tight world-aligned bounds (`BRepBndLib::AddOptimal`).
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the bounds cannot be computed.
    pub fn bounds(&self) -> Result<(Point, Point), GeomError> {
        let mut out = Vec::with_capacity(6);
        glue::bounds(&self.inner, &mut out).map_err(|error| kernel("bounds", &error))?;
        let [x0, y0, z0, x1, y1, z1] = out[..] else {
            return Err(GeomError::Kernel {
                reason: "bounds returned a malformed record".to_owned(),
            });
        };
        if out.iter().any(|v| !v.is_finite()) {
            return Err(GeomError::Kernel {
                reason: "bounds returned a non-finite value".to_owned(),
            });
        }
        Ok((Point::new(x0, y0, z0), Point::new(x1, y1, z1)))
    }

    /// The solid under a similarity given as the 12 row-major coefficients
    /// of its 3×4 matrix, geometry copied and rewritten; consumes the
    /// handle.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the matrix is singular or OCCT refuses.
    pub fn transformed(self, coefficients: &[f64; 12]) -> Result<Self, GeomError> {
        let shape = glue::transform(&self.inner, coefficients)
            .map_err(|error| kernel("transform", &error))?;
        drop(self);
        Self::from_shape(&shape, "transform")
    }

    /// Every distinct edge as a curve (lines and full circles exact, the
    /// rest discretized at `deflection`) plus the face count.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the kernel fails or its records are
    /// malformed.
    pub fn edges(&self, deflection: Deflection) -> Result<(Vec<Curve>, usize), GeomError> {
        let (mut kinds, mut counts, mut data) = (Vec::new(), Vec::new(), Vec::new());
        let faces = glue::edges(
            &self.inner,
            deflection.linear(),
            deflection.angular(),
            &mut kinds,
            &mut counts,
            &mut data,
        )
        .map_err(|error| kernel("edges", &error))?;
        let faces = usize::try_from(faces).map_err(|_| GeomError::Kernel {
            reason: format!("edges returned a negative face count {faces}"),
        })?;
        Ok((decode_curves(&kinds, &counts, &data)?, faces))
    }

    /// Every distinct vertex.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the kernel fails.
    pub fn vertices(&self) -> Result<Vec<Point>, GeomError> {
        let mut out = Vec::new();
        glue::vertices(&self.inner, &mut out).map_err(|error| kernel("vertices", &error))?;
        let (triples, ragged) = out.as_chunks::<3>();
        if !ragged.is_empty() {
            return Err(GeomError::Kernel {
                reason: "vertices returned a buffer that is not a multiple of 3".to_owned(),
            });
        }
        triples
            .iter()
            .map(|&[x, y, z]| {
                let p = Point::new(x, y, z);
                if p.0.is_finite() {
                    Ok(p)
                } else {
                    Err(GeomError::Kernel {
                        reason: "vertices returned a non-finite point".to_owned(),
                    })
                }
            })
            .collect()
    }

    /// The planar section through `frame`'s xy plane: one closed curve per
    /// loop (a single circular edge stays a circle; the rest are polylines
    /// discretized at `deflection`), edges connected into loops at
    /// `tolerance`, plus the number of TANGENT CONTACTS dropped — open
    /// chains along which the plane touches the solid without entering it
    /// (a plane tangent to a cylinder along a generatrix, a plane through
    /// one edge of a box, a plane grazing a bore's wall), which bound no
    /// region. Empty when the plane misses the solid. An open chain that
    /// is NOT a contact (the solid on one side of it) is a loop the kernel
    /// failed to close, and an error.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the section fails, a loop did not close,
    /// or the records are malformed.
    pub fn section(
        &self,
        frame: &Frame,
        tolerance: f64,
        deflection: Deflection,
    ) -> Result<Section, GeomError> {
        let plane = [
            frame.origin.0.x,
            frame.origin.0.y,
            frame.origin.0.z,
            frame.z.x,
            frame.z.y,
            frame.z.z,
        ];
        let (mut kinds, mut counts, mut data) = (Vec::new(), Vec::new(), Vec::new());
        let contacts = glue::section(
            &self.inner,
            &plane,
            tolerance,
            deflection.linear(),
            deflection.angular(),
            &mut kinds,
            &mut counts,
            &mut data,
        )
        .map_err(|error| kernel("section", &error))?;
        let contacts = usize::try_from(contacts).map_err(|_| GeomError::Kernel {
            reason: format!("section returned a negative contact count {contacts}"),
        })?;
        let loops = decode_curves(&kinds, &counts, &data)?;
        if let Some(open) = loops.iter().find(|curve| !curve.is_closed()) {
            return Err(GeomError::Kernel {
                reason: format!(
                    "section returned an open curve ({}) as a loop",
                    open.variant_name()
                ),
            });
        }
        Ok(Section { loops, contacts })
    }

    /// Write `solids` to a STEP AP214 file at `path`, declaring the
    /// document unit (`millimeters` per document unit) and a header whose
    /// every field is fixed — `name`, [`STEP_TIMESTAMP`], author and
    /// organisation `cicada` — so the same solids give the same bytes.
    /// Consumes the handles. Runs under the STEP lock.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if a transfer or the write fails;
    /// [`GeomError::BadParameter`] for an empty list.
    pub fn write_step(
        solids: Vec<Self>,
        path: &str,
        millimeters: f64,
        name: &str,
    ) -> Result<(), GeomError> {
        if solids.is_empty() {
            return Err(GeomError::BadParameter {
                name: "solids",
                value: "[]".to_owned(),
                requirement: "at least one solid to write",
            });
        }
        let compound = compound_of(solids.into_iter().map(|h| h.inner))?;
        let _guard = step_guard();
        quiet_messenger();
        glue::step_write(&compound, path, millimeters, name, STEP_TIMESTAMP)
            .map_err(|error| kernel("STEP write", &error))
    }

    /// Read every solid of a STEP file, scaled to the document unit
    /// (`millimeters` per document unit), in the file's order. Runs under
    /// the STEP lock.
    ///
    /// # Errors
    ///
    /// [`GeomError::Kernel`] if the file cannot be read or translated, or
    /// holds no solid.
    pub fn read_step(path: &str, millimeters: f64) -> Result<Vec<Self>, GeomError> {
        let shape = {
            let _guard = step_guard();
            quiet_messenger();
            glue::step_read(path, millimeters).map_err(|error| kernel("STEP read", &error))?
        };
        let count = fork::cicada_count_solids(&shape);
        if count <= 0 {
            return Err(GeomError::Kernel {
                reason: format!("STEP: `{path}` holds no solid"),
            });
        }
        (0..count)
            .map(|index| {
                let solid =
                    glue::nth_solid(&shape, index).map_err(|error| kernel("STEP read", &error))?;
                Self::from_shape(&solid, "STEP read")
            })
            .collect()
    }
}

fn positive(name: &'static str, value: f64) -> Result<(), GeomError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(GeomError::BadParameter {
            name,
            value: format!("{value}"),
            requirement: "must be finite and > 0",
        })
    }
}

fn quiet_messenger() {
    QUIET.call_once(|| {
        // A failure to lower the printers' trace level is not a failure of
        // the translation: the kernel would merely narrate on stdout.
        let _ = glue::quiet_messenger();
    });
}

/// The `tessellate` node's half of the contract: a display tessellation
/// that closed becomes the `Watertight<Mesh>` the mesh tier needs; one that
/// did not is refused with the text a user can act on — the solid, the
/// deflection, the counts — never the weld's internals.
fn closed_or_refused(display: DisplayTessellation) -> Result<Tessellation, GeomError> {
    if !display.watertight {
        return Err(GeomError::NotWatertight {
            reason: format!(
                "the kernel's mesh of this solid does not close at deflection {} / {} rad ({} \
                 faces, {} vertices, {} triangles after welding) — the solid itself may be \
                 valid; try another deflection, or keep it as a Solid (display draws it as is)",
                display.deflection.linear(),
                display.deflection.angular(),
                display.faces,
                display.mesh.vertex_count(),
                display.mesh.triangle_count()
            ),
        });
    }
    Ok(Tessellation {
        mesh: Watertight(display.mesh),
        faces: display.faces,
    })
}

/// Weld per-face vertices on bit-identical positions and drop the zero-area
/// triangles this can create. Closure is the caller's question
/// (`Mesh::is_watertight`): the node requires it, display reports it.
fn weld(positions: &[f64], indices: &[u32]) -> Result<Mesh, GeomError> {
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
    Ok(Mesh::new(welded_positions, welded_indices)?)
}

#[cfg(test)]
mod node_set_tests;
#[cfg(test)]
mod tests;
