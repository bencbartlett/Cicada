//! Transform nodes (docs/08 §Catalog 10) — kind-preserving over the `T`
//! type variable: a moved `Closed<Curve>` is still a `Closed<Curve>`,
//! statically (checker) and at runtime (the
//! [`Transformable`](cicada_core::geometry::Transformable) enum). Every
//! spike transform is a similarity, so analytic curves transform EXACTLY
//! (`cicada_geom::transform`).

#[cfg(test)]
pub(crate) mod support;

pub mod linear_array;
pub mod mirror;
pub mod r#move;
pub mod orient;
pub mod rotate;
pub mod scale;
