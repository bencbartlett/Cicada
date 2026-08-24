# The wall — example, playground, and nightly regression corpus

The wall-piece pipeline, end to end (docs/15 stage 6): the production
magnetic-field pyramid wall — 1,200 Voronoi frusta with debossed IDs and
two crush-rib pin bores each, packed onto 79 Bambu plates, plus the CNC
board DXF — rebuilt on the engine from a frozen layout and compared
against the files that were actually fabricated from.

This directory is ONE copy with three jobs (DECISIONS.md, corpus row
revised 2026-08-19: the regression corpus = every example with committed
golden outputs; the wall is the first): the full-size example, the app
playground — open it, edit freely; `git checkout -- examples/wall`
reverts the tracked files, and `git clean -f examples/wall` drops the
layout sidecar the canvas creates once you move a node
(`wall.cic.layout.json`, a new untracked file; docs/10 §The layout sidecar) — and
the pipeline the nightly job measures against production.
Engine-wide tooling (the normalizer, the measurement harness, the offline
tests) lives in the repo's `tools/`; only the wall-specific layout tools
live here under `tools/`.

## What is here

| Path | Contents |
|---|---|
| `wall.cic` | The pipeline. `--node carved` is the carve-speed criterion; `--node bambu --node dxf` writes the fabrication files (explicit runs only) |
| `scripts/` | The Python script nodes: the production GhPython cores ported verbatim (`scripts/README.md` lists every node, port, source line, and declared deviation) |
| `inputs/layout.json` | The frozen production layout, extracted from the shipped artifacts: recovered Voronoi seeds, per-cell shrink, heights, lean lengths, bins, export flags — plus the production cells/centroids/leans/IDs as checks |
| `inputs/bambu/` | The two Bambu reference projects the packer/writer harvest (process overlay, keep-outs, embedded profiles) — copied from the wall repo |
| `tools/extract_layout.py`, `tools/recover_seeds.py` | Wall-only dev tools (not pipeline nodes) that produce `inputs/layout.json` from the wall repo's exports; `recover_seeds.py` needs numpy |
| `golden/production/` | The production references: `board_postprocessed.dxf` and `manifest.csv` (copies of the shop files), `coil_manifest.csv`, per-file `plates_*.summary.json` (canonical summaries of the five 50–116 MB production 3MFs, which stay outside the repo), the extraction and seed-recovery reports |
| `golden/cicada/` | Reserved for our own golden output hashes (a normalize-time determinism check, v0.1); the nightly job compares against `golden/production/` today |
| `out/` | Where the exporters write (gitignored) |

Engine-wide, in the repo's `tools/` (not here):

| Path | Contents |
|---|---|
| `tools/normalize.py` | The output normalizer + comparer (3MF / DXF / manifest; markdown report; exit code = verdict) |
| `tools/test_*.py` | Offline unit tests for the wall scripts and the normalizer (`python -m unittest discover -s tools -p "test_*.py"`; the production cross-checks skip without the wall repo) |
| `tools/measure/` | The docs/15 measurement harness: `carve.sh`/`carve.ps1` (cold/warm carve), `slider_loop.mjs` (preview latency), `esc.mjs` (cancel time-to-idle); Node ≥ 20, no deps |

## Running it

```
# in the app (the canvas writes THIS committed copy — that is the point; revert with git)
cargo run --release -p cicada-cli -- serve examples/wall/wall.cic --web-dir web/dist
# cold carve (the criterion): a fresh cache dir, release build
cargo run --release -p cicada-cli -- run examples/wall/wall.cic --node carved --time --cache-dir <fresh-dir>
# the fabrication files (land in examples/wall/out/)
cargo run --release -p cicada-cli -- run examples/wall/wall.cic --node bambu --node dxf --cache-dir <dir>
# compare against production (report + verdict)
python tools/normalize.py all --ours examples/wall/out --ref examples/wall/golden/production --report examples/wall/out/report.md
```

Python 3 on PATH is all the scripts need (pure stdlib: the wall's
scripts never used numpy). The measurement protocol, the recorded numbers,
and their machine spec live in
[docs/15-spike-plan.md](../../docs/15-spike-plan.md) §Stage-6 results.
The first solve is the ~6.5 s cold carve (release build).

The same pipeline runs nightly in CI (`.github/workflows/nightly.yml`,
`wall-corpus` job): a release build on `ubuntu-latest`, the two exporters
over a cold cache, then `normalize.py all` against `golden/production/`;
the job's verdict IS the normalizer's exit code, and the report lands in
the run's job summary (plus a best-effort artifact). First real run,
2026-08-20 (commit 63f4212, when this directory was still `corpus/`):
**overall NOISE** — every difference the normalizer found on the Linux
build was declared noise, the same verdict the Windows dev machine gives;
that is the cross-platform evidence the golden-hash discipline rests on.
The first run from THIS layout (`examples/wall/` + `tools/`), 2026-08-22:
<https://github.com/bencbartlett/Cicada/actions/runs/32562121114> —
**overall NOISE** again (the `wall corpus end-to-end` job), and the same
verdict on the two nights after it (runs 32628110812 and 32707587367).

## What the pipeline does, honestly

`layout.json` is the frozen production layout. The pipeline consumes from
it exactly what production consumed from its own upstream: the Voronoi
SEEDS (recovered from the shipped cells by bisector least squares — every
production cell vertex is reproduced within 1.1 µm), the per-cell shrink
factors (the wall's density modulation: cells shrink with field strength),
the vertical heights and apex lean lengths (the Grasshopper graph-mapper
chain that produced them from the field is not ported; the field law is
recorded in the extraction report), the colour bins, and the export flags.
Everything else is computed: Voronoi → area centroids → shrink → the 2D
Biot–Savart field at the seeds (soft core 243.84 mm = 0.1 × 96 in, as
production) → lean directions → tip caps (the base cell scaled by 0.07 about
the apex — the printed 1.4.1 caps) → `loft` → IDs + deboss placement →
`text_solids` glyph cutters → pin cutters (bore + chamfer + 60° cone
ceiling; slot with vanes on the lean pin) → one Manifold difference per part
→ terrain packing → `orient` → the Bambu multi-plate 3MF writer and the R12
board DXF writer, both ported verbatim.

The production cells, centroids, lean directions, and IDs ride along in
`layout.json` as CHECKS: `wall_labels(ids_expected=…)` refuses if the
recomputed IDs differ; the normalizer compares the DXF pins and outlines
against the shop file.

## What was found in production while porting (recorded, not hidden)

- The packer ran with `PackStep = 2` and an H2 bed width of 320 mm; its
  apexes were computed at the Voronoi seeds (where the field was solved),
  so the shipped parts sit a median 0.57° off lean-to-+Y — `wall.cic`
  passes `apex_origins=layout.seeds` to reproduce the shipped yaw.
- The shipped X1C plates were packed WITHOUT the X1-series front-left
  bed-exclude block: the production packer searched only the export
  directory and its parent for `example_settings_x1c.3mf`, which lived in
  the repo root — `bed_exclude=False` reproduces that; `True` is right for
  a new layout.
- The 1.4.1 tip caps are the cell scaled by 0.07 about the apex, not the
  `tip_caps.py` triangles (export 1.4 used triangles).
- `board_postprocessed.dxf` declares four layers in its LAYER table but
  also carries 3,828 TEXT entities (the regen path's quirk) — reproduced
  as shipped; the 3MF zip entries are stamped with the wall clock
  (reproduced with a fixed 1980-01-01 — the one declared writer deviation);
  `I59` is listed in `coil_manifest.csv` but missing from every
  `coil_2.3mf` after export 1.4.
- 58 coil-captured parts and 5 parts dropped in 1.4.1 are `exported=false`
  (they are built and carved, never exported — like production).

## Regenerating the inputs

```
python examples/wall/tools/extract_layout.py   # production artifacts → inputs/layout.json + golden/production/*
python examples/wall/tools/recover_seeds.py    # bisector least squares; writes seeds/keep/cell_scales only if every cell vertex reproduces within 2e-3 mm
python examples/wall/tools/extract_layout.py   # second pass: re-derives the field-based fallbacks at the seeds
python tools/normalize.py summarize <prod>.3mf -o examples/wall/golden/production/plates_f<bin>_<color>_<printer>.summary.json   # per production plate file
```

The wall repo is read-only for these tools (path baked in; `--wall-repo`
overrides). Nothing here is hand-edited: the tools are deterministic and
idempotent, and every generated file carries a provenance stamp naming
the tool that wrote it (the offline suite checks those stamps point at
files in the repo). One subtlety of the two-pass protocol: the
`scale_vs_height_linear_fit` diagnostic in `seed_recovery_report.json`
is fitted against the heights `recover_seeds.py` finds in `layout.json`,
and the trimmed coil parts' fallback heights are evaluated at the
centroids on pass 1 and at the seeds on pass 2 — so a fresh two-pass run
and a re-run over an already-seeded layout differ in those four numbers
only; the seeds and `cell_scales` are byte-identical either way.
