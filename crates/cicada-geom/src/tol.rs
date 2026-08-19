//! The sanctioned float-comparison API (doc 14 §Tolerance): the ONLY float
//! comparison path in geometry code. Raw float `==`/`<` on geometric
//! quantities is lint-banned by convention; every decision point routes
//! through these helpers with an EXPLICIT tolerance from `ProjectConfig`
//! (tolerance is explicit state, never ambient — DECISIONS.md row 49).

use cicada_core::spatial::Point;

/// `|a − b| <= tol`.
#[must_use]
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// `|x| <= tol` — "is effectively zero at this tolerance".
#[must_use]
pub fn near_zero(x: f64, tol: f64) -> bool {
    x.abs() <= tol
}

/// Two points within `tol` of each other (Euclidean).
#[must_use]
pub fn coincident(a: Point, b: Point, tol: f64) -> bool {
    a.0.distance_squared(b.0) <= tol * tol
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_is_inclusive_at_the_boundary() {
        assert!(close(1.0, 1.0 + 1e-6, 1e-6));
        assert!(!close(1.0, 1.0 + 2e-6, 1e-6));
    }

    #[test]
    fn coincident_uses_euclidean_distance() {
        let a = Point::new(0.0, 0.0, 0.0);
        let b = Point::new(3e-7, 4e-7, 0.0); // distance 5e-7
        assert!(coincident(a, b, 1e-6));
        assert!(!coincident(a, b, 4e-7));
    }
}
