//! Mesh & field nodes (docs/08 §Catalog 8): the Manifold booleans over
//! watertight meshes and the `as_watertight` refinement (the mesh-tier
//! solid).

#[cfg(test)]
pub(crate) mod support;

pub mod as_watertight;
pub mod mesh_difference;
pub mod mesh_intersection;
pub mod mesh_union;
