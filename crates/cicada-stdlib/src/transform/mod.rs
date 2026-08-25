//! Transform nodes (docs/08 §Catalog 10) — kind-preserving over the `T`
//! type variable: a moved `Closed<Curve>` is still a `Closed<Curve>`,
//! statically (checker) and at runtime (the
//! [`Transformable`](cicada_core::geometry::Transformable) enum). The
//! similarity nodes (`move`, `rotate`, `rotate_axis`, `scale`, `mirror`,
//! `orient`, the three arrays) transform analytic curves EXACTLY
//! (`cicada_geom::transform::Similarity`); the two general-affine nodes
//! (`scale_nu`, `transform` over an `Xform`) carry each kind only where the
//! result is still that kind and refuse, typed, where it is not
//! (`cicada_geom::transform::Affine`). `construct_xform` and
//! `compose_xform` make and multiply the `Xform` values `transform` takes.

pub(crate) mod support;

pub mod compose_xform;
pub mod construct_xform;
pub mod linear_array;
pub mod mirror;
pub mod r#move;
pub mod orient;
pub mod polar_array;
pub mod rectangular_array;
pub mod rotate;
pub mod rotate_axis;
pub mod scale;
pub mod scale_nu;
// The node IS named `transform` and every node file is named after its
// dialect name (one node per file, DECISIONS.md stdlib row), so the
// category module and the node module share the name by rule.
#[allow(clippy::module_inception)]
pub mod transform;
