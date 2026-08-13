---
name: dialect-change
description: Change the .cic dialect — grammar, parser, minimal-edit writer, or checker — with round-trip fixtures on both sides. Use for ANY change to cicada-lang's parsing, emission, gestures, or diagnostics.
---

# Change the dialect

The dialect is locked by DECISIONS.md (grammar rules, round-trip writer
discipline, one-binding-per-line). Read those rows and docs/10 BEFORE
touching grammar; a grammar change that contradicts them revises the
ledger row explicitly in the same commit or doesn't happen.

## Invariants that never break

- **Byte-identical round-trips**: parse → emit reproduces every input
  exactly (comments, spacing, broken lines). The document model owns
  this by construction — never add a code path that re-emits a line the
  gesture didn't touch.
- **Total error recovery**: a statement that fails to parse becomes a
  `Broken` line redding ITS node; the file always parses.
- **Minimal edits**: writer gestures splice at spans. A slider drag is a
  one-token diff. Never reformat, reorder, or realign.
- **Domain-quality errors**: pointed messages ("name the intermediate"),
  never parser jargon. Every diagnostic is the doc-11 JSON shape.

## Checklist

1. **Grammar/parser change** → add corpus coverage: extend a fixture in
   `crates/cicada-lang/tests/fixtures/corpus/` so the new syntax
   round-trips, plus a parse-error case proving the loud refusal if the
   change adds one. Corpus files are NOT auto-discovered — a new file
   must also join the `include_str!` list in `tests/roundtrip.rs` and
   the parse-count assertions in `corpus_fixtures_parse_fully`.
2. **Writer/gesture change** → fixtures on BOTH sides (doc 14): a
   `before.cic` + `after.cic` pair in `tests/fixtures/gestures/<name>/`
   and a test in `tests/gestures.rs` applying the op. After must be
   byte-exact.
3. **Checker/diagnostic change** → positive + negative cases in
   `tests/checker.rs`; diagnostics snapshot via insta. Bless with
   `cargo insta review`, or equivalently
   `$env:INSTA_UPDATE = "always"; cargo test -p cicada-lang` — then
   review the .snap diff and commit it with the reason. Never edit a
   .snap by hand.
4. Checker tests use the HAND-BUILT fake catalog in `tests/checker.rs`,
   never the real stdlib — checker behavior must not churn when nodes
   are added.
5. Run `cargo test -p cicada-lang`, then the `verify-change` loop.
6. If the change alters what the canvas will render or edit (stage 5+),
   note it in the commit body — the protocol/UI side consumes these
   shapes.
