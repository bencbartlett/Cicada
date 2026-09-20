use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — a padded sub-group is not a column: `" Operators "` would register
/// beside `Operators` and the menu bar would show two columns for one. The
/// stdlib's conformance test would refuse it there (not a listed column),
/// but the macro holds the SHAPE for every registry, not only the stdlib's.
/// Everything else is in order — `# Returns` included — so the padding is
/// the ONLY reason this does not compile, and the value trims to a real
/// name, so a macro that dropped the whitespace check alone would compile
/// it (`sub_blank.rs` witnesses the emptiness half).
///
/// # Returns
///
/// The value.
#[node(category = "Maths & logic", sub = " Operators ", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
