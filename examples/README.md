# Examples — a headless playground

Annotated `.cic` pipelines exercising the stage-4 surface. Each file's
header comment carries its run commands; the general shape is:

```
cargo run -p cicada-cli -- run examples/<file>.cic [--node <name>]... [--time] [--hashes]
```

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

Notes that save a first-timer some head-scratching:

- Exporters are **effectful**: a plain run solves up to their inputs and
  skips the export; `--node dump` is the explicit action that writes the
  file. Exports are also never served from cache — they really run,
  every time.
- Everything is cached in the user cache directory: the second run of
  any example prints `… from cache` timings. Edit a slider value and
  only its cone recomputes.
- `--hashes` prints stable content hashes instead of values — two runs
  (or two machines) producing the same hashes is the determinism
  contract, testable from the shell.
- The example `.obj` outputs are gitignored; regenerate them at will.
