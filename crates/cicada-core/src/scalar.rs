//! Non-spatial scalar value types: `Domain`, `Color`, `IndexMap`.

/// A 1-D interval `start..end`. `start > end` is legal (decreasing domains
/// exist in GH practice); nodes that need an ordered domain say so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Domain {
    /// Interval start.
    pub start: f64,
    /// Interval end.
    pub end: f64,
}

impl Domain {
    /// Construct from endpoints.
    #[must_use]
    pub const fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }
}

/// Linear RGBA color, each channel f64 (f64-canonical policy, doc 14).
/// Channel range is conventionally 0..=1 but not clamped here — HDR values
/// pass through; display mapping is the viewer's concern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red.
    pub r: f64,
    /// Green.
    pub g: f64,
    /// Blue.
    pub b: f64,
    /// Alpha.
    pub a: f64,
}

impl Color {
    /// Construct from channels.
    #[must_use]
    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }
}

/// Element provenance through reordering/culling ops (docs/08 rule 6,
/// docs/09): `map[i]` is the ORIGINAL slot index of output element `i`.
/// `Sort`, `Cull`, `compact`, and friends return one of these so identity
/// survives the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexMap(pub Vec<u64>);
