//! Output & display nodes (docs/08 §Catalog 11). Display is an edge
//! (docs/08 rule 9): these are pure sinks; the viewer consumes them at
//! stage 5, and headless solves just compute their inputs. Spike
//! simplification, noted in docs/08: `custom_preview` colors with a
//! `Color` (the `Material` kind arrives with the real display pipeline).

use cicada_core::geometry::GeometryValue;
use cicada_core::scalar::Color;
use cicada_core::spatial::Plane;
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

/// Inputs for [`text_tag`].
#[derive(Ports, Clone, Debug)]
pub struct TextTagIn {
    /// Where the tag sits (the plane orients it).
    pub location: Plane,
    /// The text to show.
    pub text: String,
    /// Text height in document units.
    #[port(default = 1.0, dimension = length)]
    pub size: f64,
}

/// Text Tag — a display-only text label at a plane.
#[node(category = "Output, display & export", tier = "S", version = 1)]
pub fn text_tag(input: TextTagIn) {
    let _ = input; // pure sink; the viewer draws at stage 5
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Curve, Line};
    use cicada_core::spatial::Point;

    use super::*;

    // Sinks: the whole contract is "accepts its inputs, computes nothing".
    #[test]
    fn sinks_accept_their_inputs() {
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
        text_tag(TextTagIn {
            location: Plane::world_xy(),
            text: "part C12".to_owned(),
            size: 2.0,
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

        #[test]
        fn property_text_tag_accepts_any_input(
            text in ".*",
            size in -1.0e6..1.0e6_f64,
            ox in -1.0e6..1.0e6_f64,
        ) {
            text_tag(TextTagIn {
                location: Plane {
                    origin: Point::new(ox, 0.0, 0.0),
                    ..Plane::world_xy()
                },
                text,
                size,
            });
        }
    }
}
