//! The `panel` node.

use cicada_core::marshal::AnyValue;
use cicada_macros::{Ports, node};

/// Inputs for [`panel`].
#[derive(Ports, Clone, Debug)]
pub struct PanelIn {
    /// Anything — the panel shows whatever arrives.
    pub data: AnyValue,
}

/// Panel — display sink; shows counts and samples on the canvas.
///
/// # Examples
///
/// ```cic
/// nums = series(start=0.0, step=2.5, count=4)
/// readout = panel(data=nums)
/// ```
#[node(category = "Params & input", tier = "S", version = 1, gh = "Panel")]
pub fn panel(input: PanelIn) {
    // Pure sink: display happens at the viewer (display is an edge,
    // docs/08 rule 9); headless solves just pull the input.
    let _ = input;
}

#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn panel_accepts_any_value_kind() {
        for data in [
            ValueData::Number(1.5),
            ValueData::Text(std::sync::Arc::from("hello")),
            ValueData::Nothing,
        ] {
            panel(PanelIn {
                data: AnyValue(HashedValue::new(data).unwrap()),
            });
        }
    }

    proptest::proptest! {
        // The panel sink is total: any sealed value is accepted.
        #[test]
        fn panel_property_total(x in -1.0e12..1.0e12_f64) {
            panel(PanelIn {
                data: AnyValue(HashedValue::new(ValueData::Number(x)).unwrap()),
            });
        }
    }
}
