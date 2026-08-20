//! Output, display & export nodes (docs/08 §Catalog 11).
//!
//! Display is an edge (docs/08 rule 9): `custom_preview` and `text_tag`
//! are pure sinks; the viewer consumes them (stage 5), and headless solves
//! just compute their inputs. Spike simplification, noted in docs/08:
//! `custom_preview` colors with a `Color` (the `Material` kind arrives
//! with the real display pipeline).
//!
//! Exporters are effectful, explicit-run only (doc 10 §7, doc 12): they
//! bind like any node but NEVER auto-run — `cicada run` skips effectful
//! leaves unless named with `--node`, and the scheduler never serves them
//! from cache (a cache hit that skipped writing the file would be a
//! silent lie).
//!
//! The text nodes set glyph outlines and glyph solids from a BUNDLED font
//! — the docs/08 open question ("bundle fonts vs system lookup") resolved
//! for reproducibility: the same `.cic` must produce the same bytes on
//! every machine, so the font travels inside the binary. The spike
//! bundles exactly one face, `DejaVu Sans Bold`
//! (`crates/cicada-stdlib/fonts/`, license alongside); more arrive with
//! v0.1. Glyph geometry, layout, and the hole-aware prism builder live in
//! `cicada_geom::text`. `size` is the CAPITAL-LETTER HEIGHT (cap height)
//! — how fabrication text is specified on drawings — not the em size.
//! Text runs from the plane's origin along +x with the baseline on the x
//! axis; `\n` stacks lines downward by `line_gap × size`, left-aligned.
//! Kerning is ignored.

mod support;

pub mod custom_preview;
pub mod export_obj;
pub mod text_outlines;
pub mod text_solids;
pub mod text_tag;

pub use support::bundled_font_names;
