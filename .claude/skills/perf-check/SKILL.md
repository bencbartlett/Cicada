---
name: perf-check
description: Run and interpret Cicada's performance benchmarks against the doc-15 targets. Use whenever a change might affect solve/kernel performance, when recording numbers for a stage gate, or when a benchmark regresses.
---

# Performance check

Doc 15's gate is EMPIRICAL — numbers, not vibes. Every benchmark number
recorded anywhere (commit body, docs, README) carries the machine spec
and the commit hash it was measured at.

## The benchmarks

| What | Command | Target |
|---|---|---|
| Carve seam (kernel-only) | `cargo run --release -p cicada-geom --example carve_bench [parts]` | 1,500 parts in **seconds** (stage-4 DoD); exits nonzero past 10 s |
| Full-pipeline carve | `cicada run corpus/wall.cic --node carved --time`, cold cache | **< 10 s** cold, < 100 ms warm (doc 15); MEASURED 2026-08-19: cold **6.5 s**, warm **0.13 ms** on the i7-13700KF |
| Solve overhead | `cicada run examples/03-voronoi.cic --time --cache-dir <fresh>` | wall ≈ dominated by booleans; warm rerun ≈ zero computed |

Baseline on the dev machine (2026-08-18, commit of stage 4): carve_bench
1,500 parts = **0.089 s carve** (0.059 ms/part, ~264k result triangles).

Stage-6 measurement harness (`corpus/measure/`, doc 15 §Stage-6 results):
`carve.sh`/`carve.ps1` (cold/warm carve), `slider_loop.mjs` (preview
latency from `/debug/state` `timings`), `esc.mjs` (cancel time-to-idle).
`CICADA_TRACE=1` on any `cicada run`/`serve` prints per-node phase timings
(key/hydrate/run+persist/memo) to stderr — the profiler until the real one
lands. All five doc-15 criteria PASSED at the stage-6 commit.

## Discipline

1. **Release builds only** — debug numbers are noise; the harness refuses
   nothing, so YOU refuse them.
2. **Environment**: every cargo command needs the dev-machine env
   (`CARGO_TARGET_DIR` outside Dropbox; cmake on PATH for the first
   manifold build — see AGENTS.md dev notes). Close nothing else down;
   we measure the machine as used.
3. **Cold vs warm**: `--cache-dir` a fresh temp dir for cold numbers; run
   twice in the same dir for warm. Never report one as the other.
4. **Compare like with like**: same part count, same machine, same
   commit ± the change under test. A regression claim needs both numbers
   in the report.
5. **Record**: numbers land in the commit body (and doc 15 §Measurement
   protocol when a stage gate is being filled in), with machine spec +
   date. Never edit a recorded historical number.
6. **Regression bar**: >20% slowdown on carve_bench or any doc-15
   criterion flipping from pass to fail blocks the change until
   explained (bigger meshes? kernel upgrade? real regression?).

## Known traps

- The first release build of `manifold-csg-sys` compiles the C++ kernel
  (cmake + git clone) and has a KNOWN transient FetchContent failure —
  retry once before diagnosing.
- Benchmark timings on this machine wobble ~±10% run to run; take the
  best of three for comparisons, and say so.
- `carve_bench` prints build time separately — frustum construction is
  not carve time; quote the carve number.
