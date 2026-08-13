use cicada_macros::Ports;

/// Bad — the port field has no doc comment.
#[derive(Ports, Clone, Copy)]
pub struct In {
    pub x: f64,
}

fn main() {}
