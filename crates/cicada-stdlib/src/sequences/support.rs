//! Shared by the seeded nodes: the one PRNG.

/// The `splitmix64` generator (a fixed, documented algorithm — the sequence
/// is part of every seeded node's contract and identical on every
/// platform). Advances `state` and returns the next 64-bit output.
pub(crate) fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The next draw as a unit-interval number: the high 53 bits → `[0, 1)`
/// with full f64 resolution (an exact dyadic fraction).
pub(crate) fn unit_draw(state: &mut u64) -> f64 {
    #[allow(clippy::cast_precision_loss)] // 53 bits fit an f64 mantissa exactly
    let unit = (splitmix64(state) >> 11) as f64 / 9_007_199_254_740_992.0;
    unit
}

/// A seed's PRNG state (seed bits, not magnitude).
pub(crate) const fn seed_state(seed: i64) -> u64 {
    #[allow(clippy::cast_sign_loss)]
    let state = seed as u64;
    state
}
