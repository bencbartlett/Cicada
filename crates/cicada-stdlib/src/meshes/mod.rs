//! Mesh & field nodes (docs/08 §Catalog 8): the mesh-tier primitives and
//! sweeps (`mesh_box` / `mesh_sphere` / `mesh_extrude` / `mesh_loft` — the
//! spike's watertight-mesh implementations, renamed when the OCCT-backed
//! `box` / `sphere` / `extrude` / `loft` took the bare names; DECISIONS.md
//! row 42), the Manifold booleans over watertight meshes, `tessellate` (the
//! explicit B-rep → mesh bridge) and the `as_watertight` refinement (the
//! mesh-tier solid).

#[cfg(test)]
pub(crate) mod support;

pub mod as_watertight;
pub mod mesh_box;
pub mod mesh_difference;
pub mod mesh_extrude;
pub mod mesh_intersection;
pub mod mesh_loft;
pub mod mesh_sphere;
pub mod mesh_union;
pub mod tessellate;
