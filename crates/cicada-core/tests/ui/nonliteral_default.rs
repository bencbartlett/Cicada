use cicada_macros::Ports;

/// Bad — non-literal defaults are undesigned until stage 4.
#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    #[port(default = f64::consts_something())]
    pub x: f64,
}

fn main() {}
