//! Sequences & random nodes (docs/08 §Catalog 2). The seeded nodes share
//! one PRNG (`support::splitmix64`) — explicit seeds, always (rule 2).

mod support;

pub mod jitter;
pub mod random;
pub mod range;
pub mod repeat;
pub mod series;
