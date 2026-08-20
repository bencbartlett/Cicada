use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — a node returning one bare value has no field to document its
/// `out` port: the `# Returns` section IS that port's doc line, and it is
/// missing here (one doc line per port, DECISIONS.md stdlib row).
///
/// # Examples
///
/// ```cic
/// y = bad(x=1.0)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
