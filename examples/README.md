# Examples — a playground, headless or in the app

Annotated `.cic` pipelines exercising the stage-4/5 surface. Each file's
header comment carries its headless run commands; the general shape is:

```
cargo run -p cicada-cli -- run examples/<file>.cic [--node <name>]... [--time] [--hashes]
```

Or open one in the app (stage 5): `cargo run -p cicada-cli -- serve
examples/02-solids.cic` prints a URL; the canvas edits the file in place
(sliders, wires, placed nodes all become minimal text edits, and the
layout sidecar `02-solids.cic.layout.json` appears next to it once you
move a node). Serve a scratch copy if you want to experiment without
touching the committed examples — the app writes what you do.

- **01-curves.cic** — params, analytic curves, curve division, an
  expression node.
- **02-solids.cic** — mesh-backed solids (box/sphere), a Manifold carve,
  and the debug OBJ exporter (open the written `.obj` in any mesh
  viewer — F3D, MeshLab, Blender, even VS Code extensions).
- **03-voronoi.cic** — the wall pipeline in miniature: seeded random
  points → Voronoi cells → extruded prisms (`each()` lift) → carve →
  OBJ.
- **04-field.cic** — the Python script-node host: a numpy field solver
  (`scripts/solve_field.py`) driving per-cell prism heights.
- **wall/** — the full 1,200-part production wall (`wall/wall.cic` + its
  Python script nodes, frozen layout, and the golden production
  references; `wall/README.md`). ONE copy with three jobs: the
  full-size example, the app playground — open it and edit freely;
  `git checkout -- examples/wall` reverts the tracked files, plus
  `git clean -f examples/wall` if you moved nodes (the canvas then writes
  an untracked `wall.cic.layout.json` sidecar) — and the pipeline the nightly
  regression job measures against production (DECISIONS.md corpus row:
  the corpus = every example with committed golden outputs). First solve
  is the ~6.5 s cold carve (release); the exporters (`--node bambu
  --node dxf`) write to `examples/wall/out/` (gitignored); compare with
  `python tools/normalize.py all …` (see `wall/README.md`).
- **05-script-geometry.cic** — script nodes beyond numbers
  (`scripts/pyramids.py`): Python returns watertight meshes through a
  multi-output node (`pyr.meshes`, `pyr.volumes`), and an effectful
  `-> None` node exports a CSV (`--node table`).
- **06-lists.cic** — lists 101 (docs/09): `range` → `sort` / `reverse`
  / `dispatch` / `group_by` with their index maps, the reducers
  (`mass_addition`, `average`), maths lifted with `each()` (`larger`,
  `floor`), and the strict-zip adapters made visible (`repeat`,
  `pad_last`) feeding `cull` and a sphere per kept column. Pure
  throughout — the second `--time` run is fully cached.
- **08-orbit.cic** — the time transport (docs/13 §Animation transport):
  a `cycle` (one 4 s loop in 120 frames) drives a planet around a sun
  and a moon around the planet through `rotate`. Headless there is no
  transport — `spin` evaluates at frame 0, the rest pose — and the
  second `--time` run is fully cached; in the app, press play on the
  play bar (or `Space`) and watch the first pass compute each frame once
  and the second pass play from cache; the scrubber seeks, Esc pauses.
  (07 is the B-rep bracket of the solid track.)

Notes that save a first-timer some head-scratching:

- Exporters are **effectful**: a plain run solves up to their inputs and
  skips the export; `--node dump` is the explicit action that writes the
  file (in the app: the node's Run button). Exports are also never served
  from cache — they really run, every time. Relative `path=` literals
  resolve against the pipeline's own directory (`examples/`), whichever
  directory you run or serve from.
- Everything is cached in the user cache directory — deliberately, so
  the playground demonstrates warm reopens (pass `--cache-dir <dir>` to
  keep experiments out of it; tests/CI always do). With `--time`, the
  second run of any example reports `0 computed, N from cache`. Edit a
  slider value and only its cone recomputes.
- `--hashes` prints stable content hashes instead of values — two runs
  (or two machines) producing the same hashes is the determinism
  contract, testable from the shell.
- The example `.obj` outputs are gitignored; regenerate them at will.
