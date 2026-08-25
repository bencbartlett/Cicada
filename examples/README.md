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
- **02-solids.cic** — B-rep solids, the default working mode: `box`,
  `sphere`, a `solid_difference` in the OCCT kernel, `volume`, then one
  `tessellate` for the debug OBJ exporter (open the written `.obj` in
  any mesh viewer — F3D, MeshLab, Blender, even VS Code extensions).
  The mesh tier continues under `mesh_*` names (03/04 stay on it).
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
- **07-simple-cad.cic** — simple traditional CAD (docs/01 use case 2):
  a mounting bracket as exact B-rep solids — `box` plate, `cylinder`
  boss, an `extrude`d gusset rib, one `solid_union`, a through-bore and
  a `linear_array` of mounting holes removed by one `solid_difference`;
  `volume`, a `section` through the plate (the holes come back as exact
  circles), `bounding_box`; `export_step` writes the STEP on
  `--node step`. Eight sliders to drag in the app; no fillets or
  chamfers yet (v0.2).
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
- **09-vectors.cic** — vectors 101 (docs/08 §Catalog 5, the C2a nodes):
  a ring of posts from one arm turned by `rotate_vector` and moved out
  from the centre (`range` includes both ends, so the last arm lands on
  the first post — `cull_duplicates` drops it and returns the index map),
  a probe's `closest_point` and `distance`, the algebra between the two
  (`angle`, `dot_product`, `cross_product`, `vector_length`,
  `deconstruct_vector`, `amplitude`), and a marker frame from a normal
  (`plane_normal`, `construct_vector`). Three sliders to drag in the app;
  pure throughout — the second `--time` run is fully cached.

**The rule: every example must solve.** CI runs each `examples/**/*.cic`
headlessly with a fresh cache through the same compile → lower → solve
functions as `cicada run <file>` with no `--node` (every non-effectful
leaf; the exporters' inputs solve, the exporters are never lowered, so
they never run) and requires zero checker diagnostics, zero red and zero
blocked bindings (`crates/cicada-cli/tests/examples_solve.rs`; the
failure names the example and the binding). What `run` accepts, the test
accepts — a binding answered by the memo within the solve (two nodes with
the same key) is as green as a computed one — with exactly two
differences, both deliberate:

- **Diagnostics anywhere refuse the example.** `run` gates them to the
  target cone and prints the rest as warnings; the app paints every one.
- **The working directory is not the pipeline's.** `run` and `serve`
  enter it (so exporter `path=` literals resolve against the pipeline);
  the test cannot (process-global, concurrent tests). Nothing here
  depends on it — exporters never run, and the wall's scripts resolve
  `inputs/` against their own `__file__` — and that is the rule it
  implies: **relative paths in non-effectful nodes must not rely on the
  cwd** (a script reads files beside itself, never beside the process).

Discovery is by extension, so a new example is covered the moment it is
committed and needs no registration; an example that needs Python needs
only the interpreter (the scripts here are dependency-free on purpose).
The wall is included — it solves cold in under ten seconds in debug on
the dev machine. A pipeline that is MEANT to show a red node does not
belong in `examples/`.

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
