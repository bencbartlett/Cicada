//! Point · Vector · Plane nodes (docs/08 §Catalog 5).

mod support;

pub mod amplitude;
pub mod angle;
pub mod closest_point;
pub mod construct_plane;
pub mod construct_point;
pub mod construct_vector;
pub mod cross_product;
pub mod cull_duplicates;
pub mod deconstruct_point;
pub mod deconstruct_vector;
pub mod distance;
pub mod dot_product;
pub mod plane_normal;
pub mod rotate_vector;
pub mod unit_x;
pub mod unit_y;
pub mod unit_z;
pub mod vector_2pt;
pub mod vector_length;
pub mod xy_plane;
pub mod xz_plane;
pub mod yz_plane;

pub use support::{UnitIn, VectorPairIn, WorldPlaneIn};
