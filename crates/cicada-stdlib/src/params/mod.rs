//! Params & input nodes (docs/08 §Catalog 1). Constructor bindings in the
//! dialect (`amps = slider(value=12.0, min=0.0, max=30.0)`, doc 10 §3);
//! the canvas renders them as widgets, the engine sees plain nodes.
//! Bare literals cover the "Literals" catalog row — a constant is a
//! binding, not a node.

pub mod choice;
pub mod clock;
pub mod cycle;
pub mod panel;
pub mod slider;
pub mod toggle;
