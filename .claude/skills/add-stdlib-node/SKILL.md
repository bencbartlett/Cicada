---
name: add-stdlib-node
description: Add or modify a node in cicada-stdlib end to end — #[node] function, Ports structs, table + property + determinism tests, catalog regeneration. Use for ANY node-catalog work, including editing an existing node's ports or docs.
---

# Add a stdlib node

Every stdlib node ships with table + property + determinism tests and doc
comments that feed the generated catalog (DECISIONS.md). No partial nodes.
Worked examples: `crates/cicada-stdlib/src/maths.rs` (multi-output:
`deconstruct_domain`) and `sequences.rs` (defaults + list output: `series`).

## Before writing code

1. Read `docs/generated/CATALOG.md` — know what exists; don't duplicate.
2. Read the node's row in `docs/08-standard-library.md` §Catalog. If the
   node isn't in the doc-08 catalog, that's a design addition: update
   docs/08 (and `DECISIONS.md` if it changes a locked decision) in the same
   commit, or stop and ask.
3. Nodes are **pure, deterministic, explicit-seeded** functions. No I/O, no
   ambient state, no wall-clock. `cicada-stdlib` never depends on
   `cicada-sched`. Invalid input = panic with a message (loud refusal),
   never a silent fallback.

## Checklist

1. **Input struct** with `#[derive(Ports)]` — one named field per input
   port; field names ARE the port names everywhere (Rust, catalog, canvas,
   dialect kwargs). Field doc comments become port docs (required).
   `#[port(default = 1.0)]` makes the port optional (literals only for
   now — non-literal defaults refuse until stage 4);
   `#[port(dimension = length)]`/`angle` tags unit-sensitive ports
   (DECISIONS.md units row).
2. **Node function** with `#[node(category = "…", tier = "S", version = 1)]`
   taking exactly one argument (the input struct). Doc comment first line
   is `` Title — description. `` (em dash; lowercase description, docs/08
   style) — the macro parses it into the catalog. When the docs/08 row
   gives only a signature (multi-node rows), write the one-liner yourself
   in the style of the neighbors (`add` → "sum of two numbers."). Output: return a bare
   value for single-output (`-> f64` becomes port `out`), or a
   `#[derive(Ports)]` output struct for multi-output. `version` is the
   semantic cache-key version (doc 12): bump it on ANY behavior change.
   Extra flags: `effectful` (exporters), `uses_tolerance` (DECISIONS.md
   tolerance row), `name = "…"` (dialect-name override — note `fn move_`
   auto-registers as `move`; the underscore never reaches the dialect).
3. **Placement**: `crates/cicada-stdlib/src/<category>.rs`, keeping fns in
   docs/08 row order; a multi-node row (`Add / Subtract / Multiply / …`)
   orders left-to-right. Catalog order within a category is module path
   (alphabetical), then source line — so inside one module, source order
   is catalog order.
4. **Tests, all three kinds** (see the worked examples):
   - *Table*: hand-picked cases including edges (zeros, negatives,
     extremes) and a `#[should_panic]` case for each loud refusal. A node
     whose whole input domain is valid (like `add`) has no refusals and
     needs no `#[should_panic]` case — don't invent one.
   - *Property*: a `proptest` invariant (symmetry, identity, round-trip,
     shape). If proptest ever finds a failure it writes a
     `proptest-regressions/` file: **commit it with the fix**.
   - *Determinism*: golden blake3 hash of the output built as a
     `HashedValue` (lists via `ValueData::List`). Bless by running once
     with a placeholder, copying the actual from the failure message, and
     saying so in the commit — that IS the blessed path for blake3
     constants (insta covers checker-diagnostic snapshots only).
   - Float comparison: geometry values ALWAYS use tolerance-aware asserts
     (doc 14's sanctioned API), never raw `==`. Exact `==` (under
     `#[allow(clippy::float_cmp)]`) is sanctioned in determinism/hash
     tests, and in table/property tests only where the node's contract is
     exact IEEE arithmetic — pure maths, as in the worked examples.
5. **Spec round-trip test** in `src/lib.rs` tests when the node exercises
   a new macro feature (first default, first multi-output, first
   dimension tag …): assert the registered spec's signature string.
6. **Run** `cargo test -p cicada-stdlib`.
7. **Regenerate the catalog** and commit both generated files in the same
   commit:

   ```
   cargo run -p cicada-cli -- catalog
   ```

8. **Finish with the `verify-change` skill** — fmt, clippy, workspace
   tests, catalog `--check`.

## Macro error UX

`#[node]`/`#[derive(Ports)]` refuse malformed input loudly — missing
version, bad doc line, missing field docs, multiple args, generic
structs/fns, non-literal defaults, `Option<Vec<…>>` ports (optional LISTS
have no representation yet). Compile-fail messages are snapshot-tested in
`crates/cicada-stdlib/tests/ui/`; if you change macro diagnostics,
re-bless (PowerShell):

```powershell
$env:TRYBUILD = "overwrite"; cargo test -p cicada-core --test macro_ui; Remove-Item Env:\TRYBUILD
```

and commit the .stderr diff with the reason. trybuild resolves deps
OUTSIDE the committed Cargo.lock — if these tests break with no repo
change, suspect a new syn/proc-macro2 release first.
