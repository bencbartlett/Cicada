use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

#[derive(Ports, Clone, Copy)]
pub struct Out {
    /// The value, doubled.
    pub twice: f64,
    /// The value, halved.
    pub half: f64,
}

/// Bad — a multi-output node documents each field of its output struct;
/// a `# Returns` section would be a second, unrendered place for the same
/// docs (and would silently win nothing). Refused.
///
/// # Returns
///
/// Twice and half the value.
///
/// # Examples
///
/// ```cic
/// a, b = bad(x=1.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> Out {
    Out {
        twice: input.x * 2.0,
        half: input.x / 2.0,
    }
}

fn main() {}
