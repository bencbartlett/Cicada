use cicada_macros::Ports;

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value with two defaults — the second would silently win.
    #[port(default = 1.0, default = 2.0)]
    pub x: f64,
}

fn main() {}
