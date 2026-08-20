//! The `remap` node.

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

/// Inputs for [`remap`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RemapIn {
    /// The value to remap.
    pub value: f64,
    /// The domain the value lives in.
    pub source: Domain,
    /// The domain to map it into.
    pub target: Domain,
}

/// Remap — map a value linearly from a source domain to a target domain.
/// Values outside the source domain extrapolate linearly (no clamping).
///
/// # Panics
///
/// Panics when the source domain is empty (`start == end`) — the map is
/// undefined there.
#[node(category = "Maths & logic", tier = "S", version = 1)]
#[must_use]
#[allow(clippy::float_cmp)] // exact emptiness IS the undefined case
pub fn remap(input: RemapIn) -> f64 {
    let span = input.source.end - input.source.start;
    assert!(
        span != 0.0,
        "remap: source domain {}..{} is empty",
        input.source.start,
        input.source.end
    );
    let t = (input.value - input.source.start) / span;
    (input.target.end - input.target.start).mul_add(t, input.target.start)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn remap_table_cases() {
        let unit_to_percent = RemapIn {
            value: 0.25,
            source: Domain::new(0.0, 1.0),
            target: Domain::new(0.0, 100.0),
        };
        assert_eq!(remap(unit_to_percent), 25.0);
        // Decreasing target — the wall used these constantly.
        assert_eq!(
            remap(RemapIn {
                value: 0.25,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(100.0, 0.0),
            }),
            75.0
        );
        // Outside the source: extrapolates, never clamps.
        assert_eq!(
            remap(RemapIn {
                value: 2.0,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(0.0, 10.0),
            }),
            20.0
        );
    }

    #[test]
    #[should_panic(expected = "source domain")]
    fn remap_empty_source_is_red() {
        let _ = remap(RemapIn {
            value: 1.0,
            source: Domain::new(3.0, 3.0),
            target: Domain::new(0.0, 1.0),
        });
    }

    proptest::proptest! {
        // Remap endpoints land exactly on the target endpoints.
        #[test]
        fn property_remap_endpoints(
            s0 in -1.0e3..1.0e3_f64, span in 0.001f64..1.0e3,
            t0 in -1.0e3..1.0e3_f64, t1 in -1.0e3..1.0e3_f64,
        ) {
            let source = Domain::new(s0, s0 + span);
            let target = Domain::new(t0, t1);
            proptest::prop_assert_eq!(
                remap(RemapIn { value: s0, source, target }),
                t0
            );
            let end = remap(RemapIn { value: s0 + span, source, target });
            proptest::prop_assert!((end - t1).abs() <= 1e-9 * t1.abs().max(1.0));
        }
    }

    #[test]
    fn remap_determinism_golden_hash() {
        let hash = |x: f64| {
            HashedValue::new(ValueData::Number(x))
                .unwrap()
                .hash()
                .to_hex()
        };
        // Arithmetic-exact inputs only: the output is an exact dyadic value,
        // so the bit pattern (and hash) is platform-free.
        assert_eq!(
            hash(remap(RemapIn {
                value: 0.3,
                source: Domain::new(0.0, 1.0),
                target: Domain::new(10.0, 20.0),
            })),
            "ea0bcd90a9ec4e1d49641c9e5b8503cb7ff24e682c11855cee8aa099de23476b"
        );
    }
}
