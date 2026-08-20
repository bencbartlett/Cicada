//! The `.cic` dialect (docs/10): lossless parser, minimal-edit writer, and
//! checker-lite with doc-11 diagnostics — the spike subset (doc 15 stage 2):
//! pragma, comments, bindings, kwargs-only calls, literals, `each()`,
//! expression RHS, multi-output unpack, port selection; writer gestures
//! place / wire / lift / set-param / delete / rename.
//!
//! Parsing is total: a broken statement reds ITS node, never the file.
//! Emission of untouched lines is byte-identical by construction — the
//! [`Document`](document::Document) stores raw text per line and the writer
//! splices at spans, never reformatting.
//!
//! `#off` disabled bindings are native (v0.1: parsed behind the prefix,
//! toggled by `writer::toggle_disable`). Later stages add: axis
//! annotations, `fmt`, adapters/`insert_between`, the tree-sitter grammar
//! (docs/10, doc 15).

pub mod ast;
pub mod check;
pub mod diag;
pub mod document;
pub mod parse;
pub mod writer;

pub use check::{
    BindingType, Catalog, Resolution, WireType, check, compatible, diagnostics, resolve,
};
pub use document::{DIALECT_VERSION, Document, Line};
