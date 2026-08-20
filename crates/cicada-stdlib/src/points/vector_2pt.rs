//! The `vector_2pt` node.

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::{Point, Vector};
use cicada_macros::{Ports, node};

/// Inputs for [`vector_2pt`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct Vector2PtIn {
    /// Tail point.
    pub a: Point,
    /// Head point.
    pub b: Point,
    /// Normalize the result to unit length.
    #[port(default = false)]
    pub unitize: bool,
}

/// Vector 2Pt — the vector from `a` to `b`, optionally unitized.
///
/// # Panics
///
/// Panics when `unitize` is on and the points coincide within tolerance —
/// a zero vector has no direction.
///
/// # Examples
///
/// ```cic
/// tail = construct_point(x=1.0, y=1.0, z=0.0)
/// head = construct_point(x=4.0, y=5.0, z=0.0)
/// direction = vector_2pt(a=tail, b=head, unitize=True)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Vector 2Pt",
    uses_tolerance
)]
#[must_use]
pub fn vector_2pt(config: &ProjectConfig, input: Vector2PtIn) -> Vector {
    let v = input.b.0 - input.a.0;
    if !input.unitize {
        return Vector(v);
    }
    let len = v.length();
    assert!(
        len > config.tol(),
        "vector_2pt: points coincide within tolerance ({len} apart) — \
         a zero vector has no direction to unitize"
    );
    Vector(v / len)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn vector_2pt_table() {
        let config = ProjectConfig::default();
        let v = vector_2pt(
            &config,
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: false,
            },
        );
        assert_eq!(v, Vector::new(3.0, 4.0, 0.0));
        let unit = vector_2pt(
            &config,
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: true,
            },
        );
        assert_eq!(unit, Vector::new(0.6, 0.8, 0.0));
    }

    #[test]
    #[should_panic(expected = "coincide within tolerance")]
    fn vector_2pt_zero_unitize_is_red() {
        let _ = vector_2pt(
            &ProjectConfig::default(),
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(1.0, 1.0, 1.0),
                unitize: true,
            },
        );
    }

    proptest::proptest! {
        // Unitized vectors have length 1 whenever the points differ.
        #[test]
        fn property_vector_2pt_unit_length(
            bx in 0.001f64..1.0e3,
            by in -1.0e3..1.0e3_f64,
        ) {
            let v = vector_2pt(
                &ProjectConfig::default(),
                Vector2PtIn {
                    a: Point::origin(),
                    b: Point::new(bx, by, 0.0),
                    unitize: true,
                },
            );
            proptest::prop_assert!((v.0.length() - 1.0).abs() < 1e-12);
        }
    }

    // Golden hash: exact subtraction, no unitize (sqrt-free); blessed via
    // run-once.
    #[test]
    fn vector_2pt_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        let v = vector_2pt(
            &ProjectConfig::default(),
            Vector2PtIn {
                a: Point::new(1.0, 1.0, 1.0),
                b: Point::new(4.0, 5.0, 1.0),
                unitize: false,
            },
        );
        assert_eq!(
            hash(ValueData::Vector(v)),
            "2361344b7d2889cf286ff64869f15fe205f4a324384183711c0afc0645a762ef"
        );
    }
}
