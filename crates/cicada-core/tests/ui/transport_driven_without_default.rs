use cicada_macros::Ports;

/// Bad — a transport-driven port evaluates as written headless (`cicada
/// run` has no transport), so it needs the default that makes "as written"
/// well-defined when the text omits it (frame 0, t 0).
#[derive(Ports, Clone, Copy)]
pub struct In {
    /// The current frame.
    #[port(transport_driven = frame)]
    pub frame: i64,
}

fn main() {}
