//! Test helpers shared by the curve nodes' tests.
//!
//! Golden-hash inputs stay transcendental-free (docs/14): sin/cos differ
//! in the last ulp across platform libms, so goldens are built from
//! lines/polylines/rectangles (pure arithmetic). The circle golden is
//! fine: it hashes the analytic VALUE (plane + radius, no evaluation);
//! dividing a circle would not be.

use cicada_core::config::ProjectConfig;

pub(crate) fn config() -> ProjectConfig {
    ProjectConfig::default()
}
