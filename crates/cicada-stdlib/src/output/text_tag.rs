//! The `text_tag` node.

use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

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
    use cicada_core::spatial::Point;

    use super::*;

    // Sinks: the whole contract is "accepts its inputs, computes nothing".
    #[test]
    fn sink_accepts_its_inputs() {
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
