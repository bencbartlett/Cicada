# Corpus

The wall-pipeline end-to-end test corpus (DECISIONS.md: "Test corpus = the
wall-piece pipeline"): field solver, colorizer, labels, pins, carve, packer,
and exporters from the 1,500-part production wall piece, plus golden output
hashes.

Populated in spike stage 6 (docs/15). The source material lives in the wall
project repo, which is not part of this repository — ask Ben for its location
when porting begins. Bulky inputs move to git LFS if they exceed tens of MB
(doc 14 §CI).

Layout (once populated):

- `wall.cic` — the ported pipeline slice
- `inputs/` — source data the pipeline consumes
- `golden/` — expected output hashes for the nightly comparison
