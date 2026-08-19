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
- **05-script-geometry.cic** — script nodes beyond numbers
  (`scripts/pyramids.py`): Python returns watertight meshes through a
  multi-output node (`pyr.meshes`, `pyr.volumes`), and an effectful
  `-> None` node exports a CSV (`--node table`).

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
