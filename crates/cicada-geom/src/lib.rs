//! Geometry operations and the typed seams to rented kernels (docs/03,
//! docs/14): tolerance-aware comparisons, frames, curve evaluation,
//! triangulation, mesh construction, similarity transforms, spade-backed
//! Voronoi, and Manifold-backed mesh booleans.
//!
//! The VALUE types live in `cicada-core` (dependency law); this crate is
//! the constructive layer the stdlib nodes call. Heavy kernel FFI is
//! quarantined here so iterating on other crates never rebuilds a kernel
//! binding. `unsafe` is permitted only inside FFI seam modules, each block
//! with a `// SAFETY:` comment (doc 14) — the current seams (manifold3d,
//! spade) are safe-Rust crates, so no `unsafe` exists here yet.
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
}
