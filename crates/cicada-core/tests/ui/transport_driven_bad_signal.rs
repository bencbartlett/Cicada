use cicada_macros::Ports;

/// Bad — the transport has two signals, `frame` (a cycle's quantized loop
/// frame) and `time` (the playhead in seconds); anything else is refused
/// rather than silently becoming one of them.
#[derive(Ports, Clone, Copy)]
pub struct In {
    /// The playhead.
    #[port(default = 0.0, transport_driven = seconds)]
    pub t: f64,
}

fn main() {}
