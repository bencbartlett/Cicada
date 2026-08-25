//! Geometry operations and the typed seams to rented kernels (docs/03,
//! docs/14): tolerance-aware comparisons, frames, curve evaluation,
//! triangulation, mesh construction, similarity transforms, spade-backed
//! Voronoi, Manifold-backed mesh booleans, the value-level solid API
//! ([`solid`] — the same signatures in every build) and — behind the
//! `occt` feature — the OCCT B-rep seam it runs on ([`occt`]).
//!
//! The VALUE types live in `cicada-core` (dependency law); this crate is
//! the constructive layer the stdlib nodes call. Heavy kernel FFI is
//! quarantined here so iterating on other crates never rebuilds a kernel
//! binding. `unsafe` is permitted only inside FFI seam modules, each block
//! with a `// SAFETY:` comment (doc 14) — the current seams (manifold3d,
//! spade, and the cxx-bridged OCCT binding) are safe-Rust crates, so no
//! `unsafe` exists here yet.
//!
//! The sanctioned float-comparison API lives in [`tol`]: the ONLY float
//! comparison path in geometry code (doc 14 §Tolerance). Every operation
//! that decides "coincident / degenerate / planar" takes tolerance
//! explicitly — nodes get it from `ProjectConfig` via `uses_tolerance`.

pub use cicada_core as core;

pub mod boolean;
pub mod curve;
pub mod export;
pub mod frame;
pub mod meshbuild;
#[cfg(feature = "occt")]
pub mod occt;
pub mod solid;
pub mod text;
pub mod tol;
pub mod transform;
pub mod triangulate;
pub mod voronoi;

/// One shared error type for geometry construction: every variant names
/// the offending input, honoring loud refusal (docs/08 rule 7). Nodes let
/// these surface as red-node messages.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GeomError {
    /// A frame's axes cannot span a plane (zero-length or parallel axes).
    #[error("degenerate frame: {reason}")]
    DegenerateFrame {
        /// What failed.
        reason: String,
    },
    /// A curve has no usable length for the requested operation.
    #[error("degenerate curve: {reason}")]
    DegenerateCurve {
        /// What failed.
        reason: String,
    },
    /// The operation requires a closed curve.
    #[error("curve is open ({variant}); close it first (as_closed)")]
    OpenCurve {
        /// The offending curve variant.
        variant: &'static str,
    },
    /// The operation requires a planar profile; a vertex lies off-plane.
    #[error("profile is not planar: vertex {vertex} lies {distance} from the profile plane")]
    NotPlanar {
        /// Offending vertex ordinal (tessellated).
        vertex: usize,
        /// Its distance from the plane.
        distance: f64,
    },
    /// A polygon cannot be triangulated (self-intersecting or degenerate).
    #[error("polygon is not simple: {reason}")]
    NotSimple {
        /// What failed.
        reason: String,
    },
    /// A general affine transform (`transform` over an `Xform`, `scale_nu`)
    /// cannot carry this kind exactly: a circle whose plane it stretches
    /// unevenly would be an ellipse (no such kind), a frame it skews is not
    /// a plane, and the kernel's solid transform takes a similarity only.
    /// Refused rather than approximated (the transform module's rule).
    #[error("a {kind} cannot take this transform exactly: {reason}")]
    AffineRefused {
        /// The kind that cannot be carried (`Circle`, `Plane`, `Solid`).
        kind: &'static str,
        /// Why, with the numbers.
        reason: String,
    },
    /// A parameter is outside its meaningful range.
    #[error("{name} = {value} is out of range: {requirement}")]
    BadParameter {
        /// Parameter name.
        name: &'static str,
        /// The offending value, formatted.
        value: String,
        /// What it must satisfy.
        requirement: &'static str,
    },
    /// Mesh construction refused (structural invariant).
    #[error("mesh construction: {0}")]
    Mesh(#[from] cicada_core::geometry::MeshError),
    /// The kernel refused the operation (Manifold error, duplicate Voronoi
    /// seeds, …), with the kernel's reason and the offending element when
    /// known.
    #[error("kernel refused: {reason}")]
    Kernel {
        /// The kernel's reason.
        reason: String,
    },
    /// The font has no glyph for a character of the text (text nodes).
    #[error("no glyph for {character:?} (U+{:04X}) in the font", *character as u32)]
    MissingGlyph {
        /// The character the font lacks.
        character: char,
    },
    /// A serialized form could not be written or read (a kernel's
    /// canonical bytes, e.g. OCCT `BinTools`).
    #[error("serialization: {reason}")]
    Serialization {
        /// What failed.
        reason: String,
    },
    /// A tessellation that had to be closed is not (a kernel's mesher left
    /// a boundary); refused rather than returned as a leaky mesh.
    #[error("not watertight: {reason}")]
    NotWatertight {
        /// What failed.
        reason: String,
    },
    /// A kernel operation that must yield exactly one solid body yielded
    /// another count: a cut that split its solid in two or removed it
    /// entirely, a union of disjoint operands, an empty intersection. A
    /// `Solid` is one body (docs/08 §7), so the result is refused rather
    /// than returned as a compound or as nothing.
    #[error("{}", not_one_solid_message(operation, *found))]
    NotOneSolid {
        /// The operation that produced the result (`cut`, `fuse`, `common`,
        /// a reader of bytes).
        operation: String,
        /// How many solid bodies it produced.
        found: usize,
    },
    /// The operation needs a rented kernel this build does not link (its
    /// cargo feature is off). Loud by design: a `Solid` in a build without
    /// OCCT cannot be drawn or operated on, and says so instead of falling
    /// back to anything.
    #[error(
        "{operation} needs the {kernel} kernel, which this build does not link \
         (cicada-geom feature `{feature}` is off)"
    )]
    KernelUnavailable {
        /// The kernel's name (`OCCT`).
        kernel: &'static str,
        /// The cargo feature that links it.
        feature: &'static str,
        /// The operation that was asked for.
        operation: &'static str,
    },
    /// A tessellation request finer than the `tessellate` node's budget
    /// ([`solid::TESSELLATE_MAX_FACETS_PER_TURN`]): at the part's own
    /// scale the mesher would place more facets around a full turn than
    /// the budget allows, and below that line its memory and time grow
    /// without bound (a unit sphere at the kernel's bare floor of 1e-7
    /// took 23 GB and did not finish — WP-C review). Refused before the
    /// mesher runs, with the floors for THIS part spelled out.
    #[error("{}", tessellation_budget_message(*linear, *angular, *extent, *min_linear, *min_angular))]
    TessellationBudget {
        /// The requested chord deviation (document units).
        linear: f64,
        /// The requested angular deviation (radians).
        angular: f64,
        /// The solid's largest bounding-box extent.
        extent: f64,
        /// The finest chord deviation the budget admits for this extent.
        min_linear: f64,
        /// The finest angular deviation the budget admits.
        min_angular: f64,
    },
}

/// The user-facing text of [`GeomError::TessellationBudget`]: the request,
/// the part's size, the floors, the reason and the way out.
fn tessellation_budget_message(
    linear: f64,
    angular: f64,
    extent: f64,
    min_linear: f64,
    min_angular: f64,
) -> String {
    format!(
        "tessellation finer than the budget: deflection {linear} / angle {angular} rad on a solid \
         {extent} across would mesh finer than {} facets per full turn at the part's own scale — \
         the floors for this part are deflection {min_linear:.3e} and angle {min_angular:.3e} rad \
         (the mesher's memory grows without bound below them); coarsen the request, or keep the \
         Solid exact (section, export_step)",
        solid::TESSELLATE_MAX_FACETS_PER_TURN
    )
}

/// The user-facing text of [`GeomError::NotOneSolid`]: what happened, the
/// rule, and the way out — never a glue identifier or a kernel enum value.
fn not_one_solid_message(operation: &str, found: usize) -> String {
    match found {
        0 => format!(
            "{operation} left no solid — a Solid is one body, and nothing remains (the operands \
             do not overlap the way this operation needs)"
        ),
        1 => format!("{operation} left one solid"),
        n => format!(
            "{operation} left {n} solids — a Solid is one body; change the inputs so one piece \
             remains, or build the pieces as separate solids"
        ),
    }
}
