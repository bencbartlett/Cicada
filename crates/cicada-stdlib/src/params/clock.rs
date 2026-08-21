//! The `clock` node.

use cicada_macros::{Ports, node};

/// Inputs for [`clock`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ClockIn {
    /// Multiplier on the playhead: the output is `t × speed` (negative
    /// runs backwards, 0 freezes). Distinct from the transport's own
    /// playback speed, which scales `t` for every time param at once.
    #[port(default = 1.0)]
    pub speed: f64,
    /// The playhead in seconds. Driven by the transport and hidden on the
    /// canvas; headless (`cicada run`) it evaluates as written — 0 unless
    /// the text says otherwise.
    #[port(default = 0.0, transport_driven = time)]
    pub t: f64,
}

/// Clock — unbounded time `0 → ∞` in seconds, driven by the transport and
/// uncached by design.
///
/// Volatile (docs/12 §Volatile nodes): it recomputes in every generation
/// and is never memoized — the escape hatch for animation that does not
/// loop. Its downstream is ordinary: keyed on the fresh value, so a
/// playhead that did not move recomputes nothing below it. Deterministic
/// per value — the same `t` and `speed` always give the same number,
/// which is what lets observers see the writer's animation exactly.
/// Grasshopper has no equivalent: its Timer is an ambient re-solve
/// trigger (docs/08 §Deliberately absent), and a Clock is a value.
///
/// # Returns
///
/// `t × speed`, in seconds.
///
/// # Panics
///
/// Panics when `t` or `speed` is not finite.
///
/// # Examples
///
/// ```cic
/// elapsed = clock(speed=2.0)
/// rise = elapsed * 0.5
/// ```
#[node(
    category = "Params & input",
    tier = "1",
    version = 1,
    gh = none,
    volatile
)]
#[must_use]
pub fn clock(input: ClockIn) -> f64 {
    assert!(
        input.t.is_finite(),
        "clock: t must be finite, got {}",
        input.t
    );
    assert!(
        input.speed.is_finite(),
        "clock: speed must be finite, got {}",
        input.speed
    );
    input.t * input.speed
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE multiplication is this node's contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn clock_table_cases() {
        let cases: &[(f64, f64, f64)] = &[
            (0.0, 1.0, 0.0),
            (2.5, 1.0, 2.5),
            (2.5, 2.0, 5.0),
            (3.0, -1.0, -3.0),
            (1234.5, 0.0, 0.0),
        ];
        for &(t, speed, want) in cases {
            assert_eq!(clock(ClockIn { speed, t }), want, "t={t} speed={speed}");
        }
    }

    #[test]
    #[should_panic(expected = "t must be finite")]
    fn clock_infinite_time_is_red() {
        let _ = clock(ClockIn {
            speed: 1.0,
            t: f64::INFINITY,
        });
    }

    #[test]
    #[should_panic(expected = "speed must be finite")]
    fn clock_infinite_speed_is_red() {
        let _ = clock(ClockIn {
            speed: f64::NEG_INFINITY,
            t: 1.0,
        });
    }

    proptest::proptest! {
        // Speed 1 is the identity on the playhead; speed scales linearly.
        #[test]
        fn clock_property_identity_and_linearity(
            t in 0.0..1.0e6_f64,
            speed in -1.0e3..1.0e3_f64,
        ) {
            proptest::prop_assert_eq!(clock(ClockIn { speed: 1.0, t }), t);
            proptest::prop_assert_eq!(clock(ClockIn { speed, t }), t * speed);
        }
    }

    // Golden hash: 12.5 s at speed 2 through the value model.
    #[test]
    fn clock_determinism_golden_hash() {
        let out = clock(ClockIn {
            speed: 2.0,
            t: 12.5,
        });
        assert_eq!(
            HashedValue::new(ValueData::Number(out))
                .unwrap()
                .hash()
                .to_hex(),
            "2b0e7b408d56df1bdbdcbd3dfa602ffbc9e9e224004be4e1ef45850e7b63057b"
        );
    }
}
