---
name: add-stdlib-node
description: Add or modify a node in cicada-stdlib end to end — one file per node under src/<category>/, the self-documenting node format (title line, port docs, # Returns, # Panics, # Examples, gh =), table + property + determinism tests in that file, catalog regeneration. Use for ANY node-catalog work, including editing an existing node's ports or docs.
---

# Add a stdlib node

Every stdlib node is one file in the standardized, self-documenting node
format (DECISIONS.md stdlib row, revised 2026-08-19; docs/14 §The node file
format), enforced by a conformance test and an example runner. No partial
nodes. Worked examples: `src/maths/remap.rs` (a `# Panics` contract and a
multi-line example), `src/maths/deconstruct_domain.rs` (multi-output),
`src/sequences/series.rs` (defaults + list output),
`src/output/export_obj.rs` (effectful).

## Before writing code

1. Read `docs/generated/CATALOG.md` — know what exists; don't duplicate.
   A node's `· GH: …` tag names the Grasshopper component it replaces.
2. Read the node's row in `docs/08-standard-library.md` §Catalog. If the
   node isn't in the doc-08 catalog, that's a design addition: update
   docs/08 (and `DECISIONS.md` if it changes a locked decision) in the same
   commit, or stop and ask.
3. Nodes are **pure, deterministic, explicit-seeded** functions. No I/O, no
   ambient state, no wall-clock. `cicada-stdlib` never depends on
   `cicada-sched`. Invalid input = panic with a message (loud refusal),
   never a silent fallback.

## Layout — one node per file

`crates/cicada-stdlib/src/<category>/<node>.rs`, where `<category>` is the
ribbon tab (docs/08 §Catalog) in snake_case:

| Category string in `#[node]` | Directory |
|---|---|
| `Params & input` | `params/` |
| `Sequences & random` | `sequences/` |
| `Maths & logic` | `maths/` |
| `List & axis` | `lists/` |
| `Point · Vector · Plane` | `points/` |
| `Curve` | `curves/` |
| `Surface & solid` | `solids/` |
| `Mesh & field` | `meshes/` |
| `Intersect & regions` | `intersect/` |
| `Transform` | `transform/` |
| `Output, display & export` | `output/` |

- The file is named after the DIALECT name (`solids/box.rs` for `fn box_`);
  keyword names are declared `pub mod r#box;` in the category's `mod.rs`.
- Add `pub mod <node>;` to `<category>/mod.rs` (alphabetical). Nothing else
  registers it — `#[node]` submits to `inventory`.
- Anything several nodes of a category share (an input struct like
  `maths::BinaryIn`, the bundled-font table, test helpers under
  `#[cfg(test)] pub(crate) mod testing`) lives in `<category>/support.rs`.
  Shared input structs are re-exported from `mod.rs` so public paths stay
  one level deep.
- Catalog order within a category is the dialect NAME — placement in the
  source never changes the committed catalog.

## The file, top to bottom

```rust
//! The `remap` node.

use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

/// Inputs for [`remap`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RemapIn {
    /// The value to remap.
    pub value: f64,
    /// The domain the value lives in.
    pub source: Domain,
    /// The domain to map it into.
    pub target: Domain,
}

/// Remap — map a value linearly from a source domain to a target domain.
/// Values outside the source domain extrapolate linearly (no clamping).
///
/// # Returns
///
/// The value mapped into the target domain (extrapolated outside the source).
///
/// # Panics
///
/// Panics when the source domain is empty (`start == end`) — the map is
/// undefined there.
///
/// # Examples
///
/// ```cic
/// unit = construct_domain(start=0.0, end=1.0)
/// percent = construct_domain(start=0.0, end=100.0)
/// scaled = remap(value=0.25, source=unit, target=percent)
/// ```
#[node(category = "Maths & logic", tier = "S", version = 1, gh = "Remap Numbers")]
#[must_use]
pub fn remap(input: RemapIn) -> f64 { /* … */ }

#[cfg(test)]
mod tests { /* table, property, golden hash */ }
```

1. **Input struct** with `#[derive(Ports)]` — one named field per input
   port; field names ARE the port names everywhere (Rust, catalog, canvas,
   dialect kwargs). Field doc comments become port docs (required; units
   where relevant). `#[port(default = 1.0)]` makes the port optional
   (non-literal defaults carry `default_doc = "…"`, their catalog
   rendering); `#[port(dimension = length)]`/`angle` tags unit-sensitive
   ports (DECISIONS.md units row). Multi-output nodes return a second
   `#[derive(Ports)]` struct, one documented field per output port.
2. **Node doc comment**, fixed sections in this order:
   - Line 1: `` Title — description. `` (em dash; lowercase description,
     docs/08 style; a full sentence). The macro parses it into the catalog.
     When the docs/08 row gives only a signature, write the one-liner in
     the style of the neighbours (`add` → "sum of two numbers.").
   - An optional paragraph of semantics.
   - `# Returns` — one line, REQUIRED for a node returning one bare value
     (`-> f64`, `-> Vec<Point>`, `-> Watertight<Mesh>` …): it becomes the
     doc of the `out` port in catalog.json and `/api/catalog` — one doc
     line per port, outputs included. Write it as a noun phrase like an
     input port's doc ("The sum `a + b`."). The macro refuses a
     single-output node without it, and refuses the section on a
     multi-output node (the output struct's fields carry the docs) or a
     sink (`-> ()` has no port).
   - `# Panics` — the red contract, rendered in the catalog as "Red when:".
     Write it as `Panics when <condition>` (the opener is stripped). Omit
     the section entirely for a total node like `add`.
   - `# Examples` — REQUIRED. One (or more) ```` ```cic ```` fence holding a
     self-contained `.cic` snippet: one binding per line, every kwarg named,
     concrete literals, NO `# cicada 1` header and no `#` comment lines (the
     runner adds the header; `#` lines inside the fence would read as
     rustdoc headings). It must CALL the node and end in bindings the
     headless run can solve. For an exporter, the exporter is the leaf: the
     runner solves its inputs and skips the effectful call itself, as
     `cicada run` does. The fence must be tagged `cic` — a bare fence is
     refused at compile time (rustdoc would doctest it as Rust).
3. **`#[node(category = "…", tier = "S", version = 1, gh = …)]`** — all
   four required. `gh = "Grasshopper Component Name"` is the component the
   node replaces, spelled as Grasshopper spells it (`"Number Slider"`,
   `"Domain Box"`, `"PolyLine"`); `gh = none` for a Cicada-only node
   (`as_closed`, the exporters). Choose honestly — the name feeds
   search-to-place for GH migrants. `version` is the semantic cache-key
   version (doc 12): bump it on ANY behavior change. Extra flags:
   `effectful` (exporters), `uses_tolerance` (the fn then takes
   `config: &ProjectConfig` first — DECISIONS.md tolerance row),
   `name = "…"` (dialect-name override; note `fn move_` auto-registers as
   `move` — the underscore never reaches the dialect).
4. **Tests, all three kinds**, in the file's `mod tests`:
   - *Table*: hand-picked cases including edges (zeros, negatives,
     extremes) and a `#[should_panic]` case for each loud refusal. A node
     whose whole input domain is valid (like `add`) has no refusals and
     needs no `#[should_panic]` case — don't invent one.
   - *Property*: a `proptest` invariant (symmetry, identity, round-trip,
     shape). If proptest ever finds a failure it writes
     `proptest-regressions/<category>/<node>.txt`: **commit it with the
     fix**.
   - *Determinism*: golden blake3 hash of the output built as a
     `HashedValue` (lists via `ValueData::List`). Bless by running once
     with a placeholder, copying the actual from the failure message, and
     saying so in the commit — that IS the blessed path for blake3
     constants (insta covers checker-diagnostic snapshots only). Golden
     inputs stay transcendental-free (no sin/cos-fed values — platform
     libms differ in the last ulp; see each category's `support.rs`).
   - Float comparison: geometry values ALWAYS use tolerance-aware asserts
     (doc 14's sanctioned API), never raw `==`. Exact `==` (under
     `#[allow(clippy::float_cmp)]`) is sanctioned in determinism/hash
     tests, and in table/property tests only where the node's contract is
     exact IEEE arithmetic — pure maths, as in the worked examples.
   - A test that inherently exercises two nodes (a construct/deconstruct
     round-trip) lives with the primary node and says so in a comment;
     the other file's tests say where its coverage lives.
5. **Spec round-trip test** in `src/lib.rs` tests when the node exercises
   a new macro feature (first default, first multi-output, first
   dimension tag …): assert the registered spec's signature string.
6. **Run**:

   ```
   cargo test -p cicada-stdlib                    # unit tests + the conformance test
   cargo test -p cicada-cli --test node_examples  # every # Examples snippet solves
   ```

   The conformance test (`crates/cicada-stdlib/tests/conformance.rs`)
   fails the build when any registered node lacks a title line, a
   description, a doc line on ANY port (inputs, named outputs, and the
   bare `out` via `# Returns`), a `gh` answer, or an example that calls it
   (as a whole identifier — `polyline(` is not a call of `line`).
   The runner (`crates/cicada-cli/tests/node_examples.rs`) parses,
   checks (zero diagnostics), lowers and solves every example with a
   fresh cache and reports all failing snippets in one list — it lives in
   `cicada-cli` because that is where the registry, the server's
   compile/lower path and the scheduler meet without breaking the
   dependency law.
7. **Regenerate the catalog** and commit both generated files in the same
   commit (CATALOG.md gains the `· GH:` tag; catalog.json gains `gh` and
   `examples`):

   ```
   cargo run -p cicada-cli -- catalog
   ```

8. **Finish with the `verify-change` skill** — fmt, clippy, workspace
   tests, catalog `--check`.

## Macro error UX

`#[node]`/`#[derive(Ports)]` refuse malformed input loudly — missing
version, missing `gh` (or `gh = None`/`gh = ""`), a bad doc line, missing
field docs, a single-output node without `# Returns` (or `# Returns` on a
multi-output node or a sink — these two surface as `evaluation panicked`
errors at the `#[node]` line: the check runs when the output ports are
assembled at compile time), multiple args, generic structs/fns,
non-literal defaults without `default_doc`, `Option<Vec<…>>` ports
(optional LISTS have no representation yet), a bare or non-`cic` fence in
`# Examples`, an empty or unterminated fence. Compile-fail messages are
snapshot-tested in
`crates/cicada-core/tests/ui/`; if you change macro diagnostics, re-bless
(PowerShell):

```powershell
$env:TRYBUILD = "overwrite"; cargo test -p cicada-core --test macro_ui; Remove-Item Env:\TRYBUILD
```

and commit the .stderr diff with the reason. trybuild resolves deps
OUTSIDE the committed Cargo.lock — if these tests break with no repo
change, suspect a new syn/proc-macro2 release first.
