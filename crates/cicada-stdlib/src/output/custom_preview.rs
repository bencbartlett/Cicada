//! The `custom_preview` node.

use cicada_core::geometry::GeometryValue;
use cicada_core::scalar::Color;
use cicada_macros::{Ports, node};

/// Inputs for [`custom_preview`].
#[derive(Ports, Clone, Debug)]
pub struct CustomPreviewIn {
    /// The geometry to draw (lift with `each()` for lists).
    pub geometry: GeometryValue,
    /// Display color (linear RGBA).
    #[port(default = Color::new(1.0, 1.0, 1.0, 1.0), default_doc = "white")]
    pub color: Color,
}

/// Custom Preview — display a geometry with a color; the display cost
/// shows in the profiler next to compute cost.
#[node(category = "Output, display & export", tier = "S", version = 1)]
pub fn custom_preview(input: CustomPreviewIn) {
    let _ = input; // pure sink; the viewer draws at stage 5
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Curve, Line};
    use cicada_core::spatial::Point;

    use super::*;

    // Sinks: the whole contract is "accepts its inputs, computes nothing".
    #[test]
    fn sink_accepts_its_inputs() {
        custom_preview(CustomPreviewIn {
            geometry: GeometryValue::Point(Point::new(1.0, 2.0, 3.0)),
            color: Color::new(1.0, 0.0, 0.0, 1.0),
        });
        custom_preview(CustomPreviewIn {
            geometry: GeometryValue::Curve(Curve::Line(Line {
                a: Point::origin(),
                b: Point::new(1.0, 0.0, 0.0),
            })),
            color: Color::new(1.0, 1.0, 1.0, 1.0),
        });
    }

    // Sinks return `()`, so a determinism golden is vacuous; the property
    // that matters is total acceptance — ANY valid input, no panic.
    proptest::proptest! {
        #[test]
        fn property_custom_preview_accepts_any_input(
            ax in -1.0e6..1.0e6_f64, ay in -1.0e6..1.0e6_f64, az in -1.0e6..1.0e6_f64,
            bx in -1.0e6..1.0e6_f64, by in -1.0e6..1.0e6_f64, bz in -1.0e6..1.0e6_f64,
            cr in 0.0..1.0_f64, cg in 0.0..1.0_f64, cb in 0.0..1.0_f64, ca in 0.0..1.0_f64,
            as_curve in proptest::bool::ANY,
        ) {
            let geometry = if as_curve {
                GeometryValue::Curve(Curve::Line(Line {
                    a: Point::new(ax, ay, az),
                    b: Point::new(bx, by, bz),
                }))
            } else {
                GeometryValue::Point(Point::new(ax, ay, az))
            };
            custom_preview(CustomPreviewIn {
                geometry,
                color: Color::new(cr, cg, cb, ca),
            });
        }
    }
}
