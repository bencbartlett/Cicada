//! `ProjectConfig` (doc 14 §Tolerance and units; DECISIONS.md tolerance
//! row): unit tag + tolerances, explicit state that participates in cache
//! keys — nodes declaring `uses_tolerance` get the tolerance hash folded
//! into their `NodeKey`, so changing tolerance invalidates exactly the
//! caches it affects. Never ambient.

use crate::hash::{KindTag, ValueHash, ValueHasher};

/// Document length unit. Convert/relabel machinery is v0.1 (DECISIONS.md
/// units row); the tag itself exists from stage 1 because exporters need it.
///
/// Discriminants are hashed (exporter/manifest keys later): append-only,
/// never renumber, never reuse — same contract as
/// [`KindTag`](crate::hash::KindTag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Unit {
    /// Millimeters — the document default (doc 14).
    #[default]
    Millimeter = 1,
    /// Centimeters.
    Centimeter = 2,
    /// Meters.
    Meter = 3,
    /// Inches.
    Inch = 4,
    /// Feet.
    Foot = 5,
}

/// Per-project configuration. One instance per project; the tolerance hash
/// joins the `NodeKey` of every node that declares `uses_tolerance`
/// (DECISIONS.md tolerance row).
///
/// Fields are sealed: [`ProjectConfig::new`] and `Default` are the only
/// doors, so an invalid tolerance can never reach a hash (mirroring
/// [`HashedValue`](crate::value::HashedValue)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectConfig {
    unit: Unit,
    tol: f64,
    tol_angle: f64,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            unit: Unit::default(),
            tol: 1e-6,
            tol_angle: 1e-9,
        }
    }
}

/// Configuration validation errors — refused loudly at construction, never
/// patched over.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// Tolerances must be finite and strictly positive.
    #[error("{field} must be finite and > 0, got {value}")]
    InvalidTolerance {
        /// Which field.
        field: &'static str,
        /// The rejected value, formatted (kept as text so the error stays Eq).
        value: String,
    },
}

impl ProjectConfig {
    /// Validate and construct.
    ///
    /// # Errors
    ///
    /// [`ConfigError::InvalidTolerance`] when a tolerance is NaN, infinite,
    /// zero, or negative.
    pub fn new(unit: Unit, tol: f64, tol_angle: f64) -> Result<Self, ConfigError> {
        for (field, value) in [("tol", tol), ("tol_angle", tol_angle)] {
            if !(value.is_finite() && value > 0.0) {
                return Err(ConfigError::InvalidTolerance {
                    field,
                    value: format!("{value}"),
                });
            }
        }
        Ok(Self {
            unit,
            tol,
            tol_angle,
        })
    }

    /// Document unit (consumed by exporters).
    #[must_use]
    pub fn unit(&self) -> Unit {
        self.unit
    }

    /// Distance tolerance in document units: point coincidence, closure,
    /// join threshold. Default `1e-6`.
    #[must_use]
    pub fn tol(&self) -> f64 {
        self.tol
    }

    /// Angular tolerance in radians: parallel/planar checks. Default `1e-9`.
    #[must_use]
    pub fn tol_angle(&self) -> f64 {
        self.tol_angle
    }

    /// The hash `uses_tolerance` `NodeKey`s fold in: tolerances ONLY. The
    /// unit tag deliberately stays out — a unit *relabel* (numbers
    /// untouched, DECISIONS.md units row) must not invalidate
    /// tolerance-keyed caches; unit-sensitive hashing arrives with the
    /// exporters that consume the tag.
    #[must_use]
    pub fn tolerance_hash(&self) -> ValueHash {
        ValueHasher::new(KindTag::ProjectConfig)
            .f64(self.tol)
            .f64(self.tol_angle)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_mm_1e6_1e9() {
        let config = ProjectConfig::default();
        assert_eq!(config.unit(), Unit::Millimeter);
        assert!((config.tol() - 1e-6).abs() < f64::EPSILON);
        assert!((config.tol_angle() - 1e-9).abs() < f64::EPSILON);
    }

    #[test]
    fn golden_default_tolerance_hash() {
        // Cross-run/platform determinism for the tolerance hash (it feeds
        // NodeKeys). Blessed via the run-once path.
        assert_eq!(
            ProjectConfig::default().tolerance_hash().to_hex(),
            "311714f8a0d295ce99860350885873d361ef2bad5f491e1efa63c9189947eaf0"
        );
    }

    #[test]
    fn tolerance_changes_the_hash_but_unit_does_not() {
        let default = ProjectConfig::default();
        let tighter = ProjectConfig::new(Unit::Millimeter, 1e-5, 1e-9).unwrap();
        let relabeled = ProjectConfig::new(Unit::Inch, 1e-6, 1e-9).unwrap();
        assert_ne!(default.tolerance_hash(), tighter.tolerance_hash());
        // A relabel (numbers untouched) must NOT dirty tolerance-keyed
        // caches (DECISIONS.md units row).
        assert_eq!(default.tolerance_hash(), relabeled.tolerance_hash());
    }

    #[test]
    fn bad_tolerances_refused() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                ProjectConfig::new(Unit::Millimeter, bad, 1e-9).is_err(),
                "tol={bad} must be refused"
            );
        }
    }
}
