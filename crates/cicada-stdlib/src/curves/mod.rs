//! Curve nodes (docs/08 §Catalog 6). Analytic values throughout
//! (DECISIONS.md row 41); evaluation lives in `cicada-geom`. (`area`, the
//! closed-curve measurement, is a Surface & solid node and lives in
//! `crate::solids`.)

#[cfg(test)]
pub(crate) mod support;

pub mod as_closed;
pub mod circle;
pub mod divide_curve;
pub mod line;
pub mod polyline;
pub mod rectangle;
