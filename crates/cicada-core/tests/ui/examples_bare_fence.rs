use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — a bare fence in `# Examples`: rustdoc would compile it as a Rust
/// doctest (and fail); the snippet must be tagged ```cic.
///
/// # Examples
///
/// ```
/// y = bad(x=1.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
