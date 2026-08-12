//! The `.cic` dialect: lexer, parser, AST, the minimal-edit writer, `fmt`,
//! and the checker — kind lattice, unification, axis rules, diagnostics
//! (docs/10, docs/11 diagnostic shape).
//!
//! Parser and checker share this crate because they share the AST and
//! diagnostics types and co-evolve; splitting them would put an interface
//! boundary through the highest-churn seam (doc 14).
//!
//! Stage 0 (doc 15): empty. The spike-subset parser, writer, and
//! checker-lite land in stage 2.

pub use cicada_core as core;
