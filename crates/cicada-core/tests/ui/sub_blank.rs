use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — `sub` is a name, not a blank: an empty sub-group would register a
/// node the menu bar has no column for. Everything else is in order —
/// `# Returns` included — so the empty `sub` is the ONLY reason this does
/// not compile. The shape rule has two halves (non-empty; no surrounding
/// whitespace) and this case witnesses the FIRST alone: `""` trims to
/// nothing and carries no padding, so a macro that dropped the emptiness
/// check would compile it. `sub_padded.rs` witnesses the other half; one
/// case for both (`" "`, which both halves refuse) let either be removed
/// with the suite still green.
///
/// # Returns
///
/// The value.
#[node(category = "Maths & logic", sub = "", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
