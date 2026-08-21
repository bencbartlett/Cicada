//! The `cycle` node.

use cicada_core::marshal::integer_to_number_exact;
use cicada_macros::{Ports, node};

/// Inputs for [`cycle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct CycleIn {
    /// Seconds per loop at transport speed 1. The transport reads it to
    /// place the frame (`frame = floor(t × frames / period) mod frames`),
    /// so in the app it must be a literal; the value itself never sees it.
    #[port(default = 4.0)]
    pub period: f64,
    /// Frames per loop — the quantization. A pass visits exactly these
    /// values, `0/frames … (frames − 1)/frames`, so one full loop warms
    /// every downstream cache entry and playback is then pure cache reads
    /// (docs/12). A literal in the app, like `period`.
    #[port(default = 120)]
    pub frames: i64,
    /// The current frame. Driven by the transport and hidden on the
    /// canvas; headless (`cicada run`) it evaluates as written — 0 unless
    /// the text says otherwise.
    #[port(default = 0, transport_driven = frame)]
    pub frame: i64,
}

/// Cycle — looping time `0 → 1`, frame-quantized and driven by the
/// transport, never by an ambient clock.
///
/// The value is `(frame mod frames) / frames`; the transport advances
/// `frame` at `frames / period` per second while playing (docs/13
/// §Animation transport) and the graph stays pure — the player feeds
/// values, so determinism, caching and observers' synchronized playback
/// all survive animation. Grasshopper has no equivalent: its Timer is an
/// ambient re-solve trigger (docs/08 §Deliberately absent), and a Cycle is
/// a value.
///
/// # Returns
///
/// The loop position in `0..1`: `(frame mod frames) / frames`.
///
/// # Panics
///
/// Panics when `frames` or `period` is not positive, or when `frames` does
/// not convert to a Number exactly (beyond 2^53 — the position would drift).
///
/// # Examples
///
/// ```cic
/// spin = cycle(period=4.0, frames=120)
/// angle = spin * 6.283185307179586
/// ```
#[node(category = "Params & input", tier = "1", version = 1, gh = none)]
#[must_use]
pub fn cycle(input: CycleIn) -> f64 {
    assert!(
        input.frames > 0,
        "cycle: frames must be positive, got {}",
        input.frames
    );
    assert!(
        input.period > 0.0,
        "cycle: period must be positive, got {}",
        input.period
    );
    let position = input.frame.rem_euclid(input.frames);
    match (
        integer_to_number_exact(position),
        integer_to_number_exact(input.frames),
    ) {
        (Some(position), Some(frames)) => position / frames,
        _ => panic!(
            "cycle: frames {} does not convert to a Number exactly (beyond 2^53) — the \
             position would drift",
            input.frames
        ),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // the position is exact IEEE division by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn at(frame: i64, frames: i64) -> f64 {
        cycle(CycleIn {
            period: 4.0,
            frames,
            frame,
        })
    }

    #[test]
    fn cycle_table_cases() {
        assert_eq!(at(0, 120), 0.0);
        assert_eq!(at(30, 120), 0.25);
        assert_eq!(at(119, 120), 119.0 / 120.0);
        // The loop wraps: frame `frames` is frame 0 again, and a negative
        // frame counts back from the end (rem_euclid, never a negative
        // position).
        assert_eq!(at(120, 120), 0.0);
        assert_eq!(at(150, 120), 0.25);
        assert_eq!(at(-30, 120), 0.75);
        // One frame per loop: always 0.
        assert_eq!(at(7, 1), 0.0);
    }

    #[test]
    #[should_panic(expected = "frames must be positive")]
    fn cycle_zero_frames_is_red() {
        let _ = at(0, 0);
    }

    #[test]
    #[should_panic(expected = "frames must be positive")]
    fn cycle_negative_frames_is_red() {
        let _ = at(0, -5);
    }

    #[test]
    #[should_panic(expected = "period must be positive")]
    fn cycle_zero_period_is_red() {
        let _ = cycle(CycleIn {
            period: 0.0,
            frames: 120,
            frame: 0,
        });
    }

    #[test]
    #[should_panic(expected = "exactly")]
    fn cycle_inexact_frame_count_is_red() {
        // 2^53 + 1 is the first integer an f64 cannot hold.
        let _ = at(0, (1 << 53) + 1);
    }

    proptest::proptest! {
        // The position is always in [0, 1), and the loop is periodic in
        // `frames` exactly.
        #[test]
        fn cycle_property_position_in_unit_interval_and_periodic(
            frame in -1_000_000_i64..1_000_000,
            frames in 1_i64..100_000,
        ) {
            let position = at(frame, frames);
            proptest::prop_assert!((0.0..1.0).contains(&position));
            proptest::prop_assert_eq!(at(frame + frames, frames), position);
            proptest::prop_assert_eq!(at(frame - frames, frames), position);
        }
    }

    // Golden hash: frame 37 of 120 through the value model.
    #[test]
    fn cycle_determinism_golden_hash() {
        let out = at(37, 120);
        assert_eq!(
            HashedValue::new(ValueData::Number(out))
                .unwrap()
                .hash()
                .to_hex(),
            "19f426be262f8117ad0e9f008226bf23434f28c18bf8b2b4119de33e98911bf7"
        );
    }
}
