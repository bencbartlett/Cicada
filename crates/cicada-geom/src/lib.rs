//! Geometry types, tolerance-aware operations, and the typed seams to rented
//! kernels — manifold3d, spade, curvo, lyon, `cavalier_contours`, ttf-parser
//! now; opencascade-rs in v0.2 (docs/03, docs/14).
//!
//! Heavy kernel FFI is quarantined here so iterating on other crates never
//! rebuilds a kernel binding. `unsafe` is permitted only inside FFI seam
//! modules, each block with a `// SAFETY:` comment (doc 14).
//!
//! The sanctioned float-comparison API (`approx_eq`, `coincident`,
//! `is_closed_within`) will live here: the ONLY float comparison path in
//! geometry code (doc 14 §Tolerance).
//!
//! Stage 0 (doc 15): empty. Mesh types and the Manifold/spade seams land in
//! stage 4.

// Keep the core dependency edge real from day 0 (the dependency-DAG check
// asserts layering; geometry values will be built on cicada-core types).
pub use cicada_core as core;
