//! The `line` node.

use cicada_core::geometry::{Curve, Line};
use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`line`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct LineIn {
    /// Start point.
    pub a: Point,
    /// End point.
    pub b: Point,
}

/// Line — a straight segment between two points.
#[node(category = "Curve", tier = "S", version = 1)]
#[must_use]
pub fn line(input: LineIn) -> Curve {
    Curve::Line(Line {
        a: input.a,
        b: input.b,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // constructor pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn line_constructs_as_given() {
        let l = line(LineIn {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 2.0, 3.0),
        });
        assert!(!l.is_closed());
    }

    proptest::proptest! {
        // Line is an exact pass-through constructor for ANY endpoints.
        #[test]
        fn property_line_pass_through(
            ax in -1.0e6..1.0e6_f64, ay in -1.0e6..1.0e6_f64, az in -1.0e6..1.0e6_f64,
            bx in -1.0e6..1.0e6_f64, by in -1.0e6..1.0e6_f64, bz in -1.0e6..1.0e6_f64,
        ) {
            let a = Point::new(ax, ay, az);
            let b = Point::new(bx, by, bz);
            let Curve::Line(l) = line(LineIn { a, b }) else {
                panic!("line variant")
            };
            proptest::prop_assert_eq!(l.a, a);
            proptest::prop_assert_eq!(l.b, b);
        }
    }

    #[test]
    fn line_determinism_golden_hash() {
        let l = line(LineIn {
            a: Point::new(0.0, 0.0, 0.0),
            b: Point::new(1.0, 2.0, 3.0),
        });
        assert_eq!(
            HashedValue::new(ValueData::Curve(l))
                .unwrap()
                .hash()
                .to_hex(),
            "d25432f6a628adba13074041192cdae076447dfa6b6d3a1ea798919662167107"
        );
    }
}
