# Geometry stack: build vs. rent

The governing rule: **build the house, rent the bedrock.** Kernel
robustness — surface–surface intersection, tolerant topology, fillets — is
decades of accumulated case-handling that no clean-room rewrite shortcuts.
Cicada's comparative advantage is everything *above* the kernel.

## The seam

Don't pick a kernel; pick a seam. Stage functions operate on typed values —
`Mesh`, `Solid` (B-rep), `Field` (implicit) — with pluggable backends, and
each operation routes to whatever is best at it. No single kernel's failure
modes own the pipeline, and backends are swappable per-stage.

Conversions are explicit, costed nodes (`tessellate: Solid → Mesh`,
`isosurface: Field → Mesh`), never silent coercions.

## Rent (use as-is, treat as upstream)

| Library | Role | Notes |
|---|---|---|
| **Manifold** (via `manifold3d` bindings) | Mesh booleans, offsets, hulls | Guaranteed-watertight, parallel, milliseconds; would eat the wall's 1,961-part carve (a half-hour Rhino ordeal) in seconds. The mesh workhorse |
| **spade / delaunator / kiddo** | Voronoi, Delaunay, spatial indices | Rust-native; the wall's numpy/scipy field+cell stages keep running as Python script nodes until promoted |
| **glam / curvo / lyon / cavalier_contours / i_overlay** | Vector math, NURBS curves+surfaces, 2D paths, offsets, planar booleans | The math tier under the stdlib; analytic primitives (lines, arcs, frusta) are Cicada's own easy math |
| **OCCT via `opencascade-rs`** | Procedural B-rep: extrude/revolve/loft/sweep, booleans, chamfers, modest fillets; **STEP import/export** (v0.2) | The only serious open B-rep kernel; its STEP support is the best open implementation. Bindings are young — fallback is a thin C++ shim; build123d/CadQuery remain the API prior art to imitate. Known ceilings: pathological booleans, ambitious fillets, big-model perf |
| **fidget** (implicits/SDFs) | Blends, lattices, organic forms; JIT-compiled evaluation | Pure-Rust successor to libfive by the same author. The operations that are nightmares in B-rep are one-liners here — a fillet is a smooth-minimum. Pairs naturally with mesh output; no STEP |
| **ttf-parser / rustybuzz** | Text → outlines (+ shaping) | Rust-native; replaces the wall's DimStyle-fighting glyph pipeline |
| **wasmtime** | Sandboxed WASM host for script nodes | Near-native compute; epoch preemption gives hard cancellation; a crashing script costs one node, never the engine |
| **rhino3dm / trimesh** (Python side) | .3dm read/write; mesh IO utilities | Interop via the script-node boundary, not modeling |
| **planegcs or SolveSpace's libslvs** | 2D constraint solving (deferred sketcher) | The solver is rentable; the sketcher work is UI |
| **Rhino.Compute** | Optional rescue backend | A kernel seat already owned; use only if OCCT's robustness ceiling is hit on real geometry. Demoted from "the middle path" to an escape hatch |
| **Blender (bpy)** | Photorealistic rendering | See doc 04 |

## Build (Cicada's actual code)

- The dataflow dialect parser and **shape/axis type checker**.
- The **scheduler**: hashing, caching, parallel execution, cancellation,
  disk memoization, profiling.
- The node registry + signature reflection (ports from types, via the
  `#[node]` proc-macro) and the **stdlib catalog** itself (~130 typed
  node functions; docs/08).
- The expression compiler (typed IR) and the WASM script host.
- The Tauri app: canvas, params panel, inspectors, and viewer glue
  (instancing, picking, per-node preview).
- **Fabrication exporters**, ported from the wall repo where they already
  exist and are production-proven: Bambu-flavored 3MF project writer
  (multi-plate, per-object settings, height ranges, dual-nozzle metadata),
  CNC DXF writer, manifests/trackers.
- The AI layer: prompt→node generation, provenance store, contract tests.

## Why not build the kernel (kept for the record)

- **SSI**: NURBS surfaces intersect in curves with no exact representation;
  robustness across tangencies/grazing/singularities is an unbounded long
  tail. Mesh booleans escaped via exact arithmetic on triangles (Manifold);
  the continuous domain has no such trick.
- **Tolerant topology**: B-reps are leaky by construction (edges only
  approximately on their faces, per-entity tolerances); keeping solids
  watertight through chained ops is the black art.
- **Fillets**: rolling-ball blends with radius-exceeds-face, corner
  patches, curvature cusps — the most person-decade-dense feature in CAD.
  Even Rhino is mediocre here; even Parasolid sometimes fails.
- **Economics**: Parasolid ≈ $30–100k+/yr, NDA'd, licensed to companies
  shipping products — unavailable for personal tooling. ACIS similar. C3D
  (~$10–25k/yr) is the budget commercial option if Cicada ever becomes a
  product. OCCT is free with known, mapped ceilings. Contributing fixes to
  OCCT's boolean/fillet core is kernel-engineering as a career; treat it
  as a dependency, not a project.
- Cautionary prior art: Fornjot and truck (Rust kernels-from-scratch) —
  years in, still far from parity, documented honestly by their authors.
