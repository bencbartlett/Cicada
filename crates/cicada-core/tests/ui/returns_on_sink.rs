use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — a sink returns nothing, so a `# Returns` section documents a port
/// that does not exist. Refused rather than silently dropped.
///
/// # Returns
///
/// Nothing — it is a sink.
///
/// # Examples
///
/// ```cic
/// shown = bad(x=1.0)
/// ```
#[node(category = "Output, display & export", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) {
    let _ = input;
}

fn main() {}
