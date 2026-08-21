//! Surface & solid nodes (docs/08 §Catalog 7): the OCCT-backed B-rep
//! primitives, sweeps, booleans and measurements — the default working
//! mode (DECISIONS.md row 42; v0.1 item 3 WP-C). The spike's mesh-backed
//! `box` / `sphere` / `extrude` / `loft` continue under `mesh_*` names in
//! `meshes/`. `area` is the closed-curve measurement of this category.

#[cfg(test)]
pub(crate) mod support;

pub mod area;
pub mod bounding_box;
pub mod r#box;
pub mod cone;
pub mod cylinder;
pub mod extrude;
pub mod extrude_to_point;
pub mod loft;
pub mod pipe;
pub mod revolve;
pub mod solid_difference;
pub mod solid_intersection;
pub mod solid_union;
pub mod sphere;
pub mod sweep;
pub mod volume;
