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
| **OCCT via `opencascade-rs`** (Ben's fork, pinned rev; see §The OCCT seam as built) | Procedural B-rep: extrude/revolve/loft/sweep, booleans, chamfers, modest fillets; **STEP import/export** (from v0.1 — Solid is B-rep-backed) | The only serious open B-rep kernel; its STEP support is the best open implementation. Bindings are young — the fork carries reviewed patches and a Cicada-specific, exception-safe glue layer (the "thin C++ shim" in practice); build123d/CadQuery remain the API prior art to imitate. Known ceilings: pathological booleans, ambitious fillets, big-model perf |
| **fidget** (implicits/SDFs) | Blends, lattices, organic forms; JIT-compiled evaluation | Pure-Rust successor to libfive by the same author. The operations that are nightmares in B-rep are one-liners here — a fillet is a smooth-minimum. Pairs naturally with mesh output; no STEP |
| **ttf-parser / rustybuzz** | Text → outlines (+ shaping) | Rust-native; replaces the wall's DimStyle-fighting glyph pipeline |
| **wasmtime** | Sandboxed WASM host for script nodes | Near-native compute; epoch preemption gives hard cancellation; a crashing script costs one node, never the engine |
| **rhino3dm / trimesh** (Python side) | .3dm read/write; mesh IO utilities | Interop via the script-node boundary, not modeling |
| **planegcs or SolveSpace's libslvs** | 2D constraint solving (deferred sketcher) | The solver is rentable; the sketcher work is UI |
| **Rhino.Compute** | Optional rescue backend | A kernel seat already owned; use only if OCCT's robustness ceiling is hit on real geometry. Demoted from "the middle path" to an escape hatch |
| **Blender (bpy)** | Photorealistic rendering | See doc 04 |

## The OCCT seam as built (2026-08-20, docs/17 Item 3 WP-A)

The probe (`docs/probes/occt-2026-08.md`) decided the shape; this is what
shipped. Everything below lives behind the `occt` Cargo feature of
`cicada-geom` (default OFF — default builds compile no C++ and link no
OCCT) in `crates/cicada-geom/src/occt/`.

- **Prebuilt, never the source build.** The binding links a prebuilt
  OpenCASCADE 7.8.1 found through `DEP_OCCT_ROOT`: conda-forge's `occt`
  build 103 ("novtk") for win-64 / linux-64 / osx-64 / osx-arm64, plus
  the run-time packages its shared libraries load (conda builds OCCT
  with FreeType/FreeImage on, and `TKDESTEP → TKXCAF → TKV3d → TKService
  → freetype + FreeImage + codecs`; every test binary links the STEP
  toolkits, so the closure is needed from day one). `tools/fetch_occt.py`
  fetches it, pinned file by file by sha256 in
  `tools/fetch_occt_manifest.json`, into the user cache dir (never a
  repo), verifies the extracted version is exactly 7.8.1 (the binding's
  own check accepts any 7.x ≥ 7.8), checks the import closure statically
  (PE/ELF/Mach-O), and prints the environment: `DEP_OCCT_ROOT`
  (`<prefix>/Library` on Windows, `<prefix>` elsewhere), the loader path
  (`PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH`), and
  `CMAKE_POLICY_VERSION_MINIMUM=3.5` for cmake-4 hosts. The shipped
  binary's own-built OCCT (FreeType/FreeImage off) is WP-C's work and a
  fetch-table change here, not a redesign.
- **The binding is Ben's fork**, `github.com/bencbartlett/opencascade-rs`
  branch `cicada`, pinned by rev in the workspace `Cargo.toml`: upstream
  `d114250` plus exactly the reviewed commits — the MSVC handle aliases
  (the source part of upstream PR #230, closed unmerged), the honest
  `BinTools`/`BRepTools` writers (the binary writer used to be the text
  writer through a cxx symbol collision; the path-only overloads
  defaulted to `theWithTriangles = TRUE`) with explicit flags and pinned
  format versions, in-memory serialization, the exception boundary, the
  `cicada` glue the seam calls, the removal of the OCCT source submodule
  and the dead in-tree `occt-sys` crate (cargo checks out a git
  dependency's submodules unconditionally, features or not: a fresh
  machine cloned OpenCASCADE's full history — 161 MB db + a 421 MB,
  36,012-file checkout — for a directory nothing read; the fork now costs
  6 MB in `~/.cargo/git`), and `LGPL-2.1-only` manifests (the current
  SPDX id for what upstream's deprecated bare `LGPL-2.1` meant). Cicada
  depends on `opencascade-sys` only (not the high-level crate, so
  `kicad-parser` and a second `glam` stay out of the graph). Patches are
  read line by line and carry their provenance in the commit message;
  blind merges of upstream PRs never.
- **What is wrapped (WP-A's set).** `occt::Solid` — a `TopoDS_Shape` that
  IS one solid; single-solid compounds (what `BRepAlgoAPI_Cut` returns)
  are unwrapped at construction and anything else refused — with
  `box_at`, `extrude_polygon` (validated with the mesh tier's planarity
  and simplicity rules and the explicit tolerance, because OCCT accepts
  collinear points and returns a zero-volume solid), `difference` (a cut
  that splits or empties the solid is refused, not returned as a
  compound), `tessellate → Watertight<Mesh>` (absolute linear + angular
  deflection; per-face nodes welded on bit-identical positions, `-0.0 →
  0.0`; zero-area triangles dropped; `is_watertight` required), and
  `canonical_bytes` / `from_canonical_bytes`. Errors are `GeomError`
  (+ `Serialization`, `NotWatertight`). WP-C adds the rest of the node
  set on the same pattern: one glue function per kernel operation,
  declared `Result`.
- **Exception policy.** OCCT's `Standard_Failure` does not derive from
  `std::exception`; cxx's default handler lets it unwind into Rust and
  the process dies (`0xC0000409`, probe `throw`). The fork's
  `bindings_common.hxx` defines the `rust::behavior::trycatch` hook that
  catches it, so every bridge function declared `Result` returns
  `Err(cxx::Exception)` with `<DynamicType>: <message>`; `std::exception`
  gets its `what()`, and a final `catch (...)` makes the boundary total
  ("unknown C++ exception") rather than an inventory of known types.
  Failures OCCT reports by status (boolean error reports, an unfinished
  mesher, a face without triangulation) are thrown on the C++ side. The
  seam calls only `Result`-declared functions. Tested per build, not per
  header: a real `Standard_DomainError` (0×0×0 box) arrives as an error,
  and the fork's `cicada_selftest_throw(kind)` drives one exception of
  each kind (OCCT, std, a thrown `int`) through the boundary — with the
  catch-all removed the last one kills the test process (`0xC0000409`).
- **Canonical bytes.** `BinTools` at the PINNED format version 4 (never
  `_CURRENT`), `theWithTriangles = false`, `theWithNormals = false`, and
  the per-shape `Free` / `Modified` / `Checked` flags normalized (they
  are history, not geometry: one mesh pass flips a box's faces to
  `Checked = 0` and changes the "without triangles" bytes — found while
  building this) on a snapshot restored afterwards. Byte-stable across
  processes and two independent OCCT builds (probe Q2), a fixed point
  under read → write, unaffected by tessellation; golden blake3 hashes
  for the transcendental-free box and prism are in the seam's tests.
  Cross-OS identity is measured by the nightly `occt (<os>)` jobs; until
  the three agree the goldens are per-OS — DECISIONS.md rows 16 and 42,
  revised 2026-08-20 at the merge of WP-A from the probe memo's §4d
  drafts (`docs/probes/occt-2026-08.md`): they record the fork and its
  patch stack, the pinned format version, the single-solid unwrapping,
  the flag normalization and the per-OS goldens rule.
- **Threads.** `occt::Solid` is `Send`, not `Sync`, and the hazard is
  wider than one solid: OCCT results SHARE `TShape`s with their inputs (a
  boolean reuses the faces it did not touch), `tessellate` attaches
  triangulation and flips `Modified`/`Checked` on them, `canonical_bytes`
  rewrites and restores `Free`/`Modified`/`Checked` — so `box` serialized
  on one thread while `box − prism` is tessellated on another (the rayon
  wavefront's sibling-node shape) is a C++ data race with no `Sync` in
  sight; measured: the INPUT's canonical bytes come back wrong. Every
  kernel call in the seam therefore runs under one process-wide kernel
  lock (`from_shape` takes the guard as proof; welding runs outside it),
  which is what makes `Send` sound. It serializes all OCCT work in the
  process — acceptable while only the seam's tests call it. WP-B's
  sharing model replaces it on purpose: deep copies at the seam
  (`BRepBuilderAPI_Copy`, so no two `Solid`s share `TShape`s) or doc 12's
  kernel worker that owns all OCCT state.

## Build (Cicada's actual code)

- The dataflow dialect parser and **shape/axis type checker**.
- The **scheduler**: hashing, caching, parallel execution, cancellation,
  disk memoization, profiling.
- The node registry + signature reflection (ports from types, via the
  `#[node]` proc-macro) and the **stdlib catalog** itself (~130 typed
  node functions; docs/08).
- The expression compiler (typed IR) and the WASM script host.
- The web app (canvas, params panel, inspectors, viewer glue —
  instancing, picking, per-node preview) and the engine server it
  talks to; the desktop app is a thin Tauri wrapper around both,
  later (web-first, doc 04).
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
