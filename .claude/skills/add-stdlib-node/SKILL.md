---
name: add-stdlib-node
description: Add or modify a node in cicada-stdlib end to end — node function, spec/registration, table + property + determinism tests, catalog regeneration. Use for ANY node-catalog work, including editing an existing node's ports or docs.
---

# Add a stdlib node

Every stdlib node ships with table + property + determinism tests and doc
comments that feed the generated catalog (DECISIONS.md). No partial nodes.

## Before writing code

1. Read `docs/generated/CATALOG.md` — know what exists; don't duplicate.
2. Read the node's row in `docs/08-standard-library.md` §Catalog. If the
   node isn't in the doc-08 catalog, that's a design addition: update
   docs/08 (and `DECISIONS.md` if it changes a locked decision) in the same
   commit, or stop and ask.
3. Nodes are **pure, deterministic, explicit-seeded** functions. No I/O, no
   ambient state, no wall-clock. `cicada-stdlib` never depends on
   `cicada-sched`.

## Checklist (stage-0/1 form)

1. **Node function** in `crates/cicada-stdlib/src/<category>.rs`, with a
   doc comment: title line (`Add — sum of two numbers.` → title "Add"),
   description, and (once the dialect lands) a runnable example.
   **The ABI is struct-in/struct-out** (DECISIONS.md): one input struct
   with named fields (`AddIn { a, b }` → `fn add(input: AddIn)`), even
   before the macros exist. Bare return is allowed for single-output nodes.
2. **Spec + registration.** Until `#[node]` lands (stage 1): hand-write the
   input struct AND the `NodeSpec` static next to the function (kept in
   sync by hand — same field names, same order) and add the spec to
   `registry()` in `src/lib.rs`. After stage 1: `#[node(category = "...")]`
   + `#[derive(Ports)]` reflect the struct instead; field names ARE the
   port names everywhere — Rust, catalog, canvas, dialect kwargs. A field
   with a default is an optional port. Single output = port named `out`.
3. **Tests, all three kinds** (see `maths.rs` for the worked example):
   - *Table*: hand-picked cases including edges (zeros, negatives, extremes).
   - *Property*: a `proptest` invariant (symmetry, identity, bounds, …).
   - *Determinism*: golden blake3 hash of the output for fixed inputs.
     To bless the initial hash: run once, copy the actual from the failure
     message, and say so in the commit — this IS the stage-0 blessed path
     (insta arrives stage 2). Raw float `==` is allowed ONLY here.
   - If proptest ever finds a failure, it writes a
     `proptest-regressions/` file: **commit it with the fix** (it is a
     regression test, not noise).
4. **Run** `cargo test -p cicada-stdlib`.
5. **Regenerate the catalog** and commit the diff in the same commit:

   ```
   cargo run -p cicada-cli -- catalog
   ```

6. **Finish with the `verify-change` skill** — fmt, clippy, workspace
   tests, catalog `--check`.

## Naming

Dialect names are snake_case (`divide_curve`). When the natural name is a
Rust keyword, only the **Rust fn** takes a trailing underscore (`fn move_`);
the `NodeSpec.name` — what the dialect, catalog, and canvas show — stays
clean (`move`, per docs/10). Categories come from
`cicada_core::catalog::CATEGORY_ORDER` (docs/08 section names) — the
registry test rejects unknown categories.
