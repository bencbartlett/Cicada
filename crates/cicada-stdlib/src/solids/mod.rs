//! Surface & solid nodes (docs/08 §Catalog 7). Doc 15's honest shim,
//! stated here too: spike `extrude`/`loft`/`box`/`sphere` carry their
//! v0.1 names but are mesh-backed — they return `Watertight<Mesh>` (the
//! mesh-tier solid), not the B-rep `Solid` that arrives with OCCT in v0.1.
//! The wall corpus is mesh-destined, so nothing in the spike's criteria
//! needs more. `area` is the closed-curve measurement of this category.

#[cfg(test)]
pub(crate) mod support;

pub mod area;
pub mod r#box;
pub mod extrude;
pub mod loft;
pub mod sphere;
