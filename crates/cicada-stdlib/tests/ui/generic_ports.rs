use cicada_macros::Ports;

/// Bad — generic port structs are a stage-4 feature.
#[derive(Ports, Clone, Copy)]
pub struct In<T> {
    /// Value.
    pub x: T,
}

fn main() {}
