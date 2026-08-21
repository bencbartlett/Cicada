//! Intersect & regions nodes (docs/08 §Catalog 9): Voronoi (the wall's
//! partition, via the spade seam in `cicada-geom`) and `section`, the
//! planar section of a B-rep solid (the OCCT seam; v0.1 item 3 WP-C).

pub mod section;
pub mod voronoi;
