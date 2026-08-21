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
- **What is wrapped (WP-A's set, WP-B's shape).** Two levels. The
  KERNEL level, `occt::Handle` — a `TopoDS_Shape` that IS one solid;
  single-solid compounds (what `BRepAlgoAPI_Cut` returns) are unwrapped
  at construction and anything else refused — with `box_at`,
  `extrude_polygon` (validated with the mesh tier's planarity and
  simplicity rules and the explicit tolerance, because OCCT accepts
  collinear points and returns a zero-volume solid), `difference` (a cut
  that splits or empties the solid is refused, not returned as a
  compound), `tessellate` (absolute linear + angular deflection; per-face
  nodes welded on bit-identical positions, `-0.0 → 0.0`; zero-area
  triangles dropped; `is_watertight` required; the face count rides
  along), `face_count`, `canonical_bytes` / `from_canonical_bytes`, and
  the two doors to the value model, `from_value` / `into_value`. The
  VALUE level, `cicada_geom::solid` — `core::Solid` in, `core::Solid`
  out: `box_at`, `extrude_polygon`, `difference`, `tessellate →
  Tessellation { mesh: Watertight<Mesh>, faces }`, plus `Deflection`
  (validated; `Deflection::display(&ProjectConfig)` is the display
  policy below) and `kernel_available()`. Its signatures exist in EVERY
  build; without the `occt` feature each returns the typed
  `GeomError::KernelUnavailable { kernel, feature, operation }` — a loud
  refusal, never a mesh-tier fallback. Stdlib nodes (WP-C) and the
  server's display path use the value level only. Errors are
  `GeomError` (+ `Serialization`, `NotWatertight`, `KernelUnavailable`).
  WP-C adds the rest of the node set on the same pattern: one glue
  function per kernel operation, declared `Result`, one value-level
  function over it.
- **The `Solid` value (WP-B).** `core::Solid` holds the canonical bytes
  (`Arc<[u8]>`) and nothing else; core checks the `BinTools` V4 header
  (`SOLID_CANONICAL_HEADER`, the same constant the seam pins as
  `CANONICAL_FORMAT_VERSION = 4`) and leaves the rest to the kernel. Its
  hash is KindTag `Solid` over the length-prefixed bytes, like every
  value; the store keeps the bytes verbatim (`StoredValue::Solid`); the
  Python boundary refuses it with a typed "not marshallable yet"; the
  checker admits it into `T` ports and display sinks
  (`TRANSFORMABLE_KINDS` / `GEOMETRY_KINDS`), with the kernel-backed
  transforms themselves WP-C's — until then `Similarity::apply` on a
  Solid is a red node saying so.
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
- **The sharing model (decided 2026-08-20, WP-B; DECISIONS.md row 16
  revised the same day).** The hazard WP-A found: OCCT results SHARE
  `TShape`s with their inputs (a boolean reuses the faces it did not
  touch), `tessellate` attaches triangulation and flips
  `Modified`/`Checked` on them, `canonical_bytes` rewrites and restores
  `Free`/`Modified`/`Checked` — so `box` serialized on one thread while
  `box − prism` is tessellated on another (the rayon wavefront's
  sibling-node shape) was a C++ data race with no `Sync` in sight;
  measured: the INPUT's canonical bytes came back wrong. WP-A's
  process-wide kernel lock serialized all OCCT work. WP-B replaces it
  with **op-local, linear handles** — option (b) of the plan, the deep
  copy obtained from the path every value already takes instead of
  `BRepBuilderAPI_Copy`:
  1. A `core::Solid` IS its bytes; a kernel handle is a derived,
     op-local artifact. Every `occt::Handle` exclusively owns its
     `TShape` graph: it is born from a constructor or from a `BinTools`
     read of canonical bytes, which builds a fresh object graph. No two
     live handles share a `TShape`.
  2. Kernel operations CONSUME their handles (`self` by value).
     `difference(self, cutter)` returns a result that shares untouched
     faces with inputs that no longer exist, so it owns its graph alone;
     `tessellate(self)` consumes because the mesher attaches its
     triangulation and a later mesh at a coarser deflection would keep
     the finer one (`BRepMesh` reuses a triangulation that already
     satisfies the request) — a re-used handle would make a `tessellate`
     result depend on what was displayed before it; booleans raise the
     tolerances of input sub-shapes in place when the intersection needs
     it (`BOPAlgo_Builder`'s default, non-`NonDestructive` mode) — a
     re-used input would make the NEXT operation depend on the previous
     one. Consumption ends both hazards by type, not by convention.
  3. Results go back to bytes (`into_value`) and the handle dies; the
     next node reads the bytes again. A warm solve computes exactly what
     a cold one does (`a_warm_difference_equals_a_cold_one`).
  The kernel lock is retired: the glue's calls touch no OCCT global
  (`BRepPrimAPI_*`, `BRepBuilderAPI_*`, `BRepAlgoAPI_Cut` with
  `RunParallel` off, `BRepMesh_IncrementalMesh` with `isInParallel` off,
  `BinTools_ShapeSet`, `TopExp_Explorer` keep their state in locals;
  `Standard_Type` registration and the memory manager are thread-safe in
  OCCT 7.x; the statics read — `BRepLib::Precision`,
  `BOPAlgo_Options::GetParallelMode` — are never written). The one OCCT
  subsystem known to keep mutable globals is `Interface_Static` (the
  STEP reader/writer parameters): WP-C's `import_step` / `export_step`
  run those calls under a lock of their own. Proof as shipped:
  `related_solids_are_safe_across_rayon_workers` — 8 rayon threads,
  13 related values (a block, six cutters through it, their six
  differences), 1,560 tasks that re-serialize, tessellate or recompute a
  difference by index, every result equal to its single-threaded
  golden — and, under the scheduler itself,
  `cicada-server/tests/solid_scheduler.rs`: a `SolveGraph` whose nodes
  are closures over `cicada_geom::solid` (a block; 48 cutters by
  `each()`; two difference nodes cutting the SAME block bytes at once;
  the block and every hole tessellated), solved on 8 threads with the
  fan-out spread into ≥ 8 chunks, every output hash equal to the
  1-thread run's and to the direct computation. `Handle` is `Send`, not
  `Sync`, and the second half is a COMPILE-TIME assertion beside the
  type (`occt/mod.rs`): `canonical_bytes(&self)` rewrites and restores
  `TShape` flags through a shared reference, sound only while no other
  thread can hold a `&Handle` — a fork revision that added
  `unsafe impl Sync for TopoDS_Shape` would fail the build instead of
  reopening the race. `core::Solid` is `Send + Sync`.
  **Determinism across heap states (review question, answered).**
  OCCT's booleans and mesher iterate maps keyed by `TShape` ADDRESS
  (`TopTools_ShapeMapHasher`), so the canonical bytes could in principle
  follow the heap. Measured otherwise on the richest corpus the seam can
  build today — `canonical_bytes_do_not_depend_on_heap_state_or_thread`:
  the block minus six slots minus a channel crossing all six (seven
  cuts, each intersecting the last; 58 faces), computed cold, after
  deterministic allocator churn under five seeds, and 24 times on 8
  threads each under its own churn — byte-identical bytes and equal
  tessellations every time. Evidence, not proof: WP-C's loft / revolve /
  multi-body booleans must rerun this shape of test on their own
  corpus before blessing goldens (the per-OS golden policy in
  DECISIONS.md row 42 hedges across OSes; this hedges across runs).
- **No handle cache (WP-B, measured).** The plan asked for a cache of
  reconstructed handles keyed by value hash so a chain of nodes would not
  re-read bytes at every step. Under the semantics above a cached handle
  is pristine only until its first kernel operation (rule 2), so such a
  cache could only ever serve the FIRST use of a value — and the re-read
  it would save is small next to the work it feeds. Measured with
  `cargo run --release -p cicada-geom --features occt --example
  solid_bench` (2026-08-20, Windows, release, single-threaded unless
  stated; the probe's 10 × 20 × 30 block and its 4 × 6 cutter, one pair
  per part):

  | step (per part) | 1 part | 100 parts | 1,000 parts |
  |---|---|---|---|
  | `box` construct + serialize (4,494 B) | 73 µs | 68 µs | 67 µs |
  | `extrude` construct + serialize (2,303 B) | 505 µs | 130 µs | 146 µs |
  | read block, bytes → handle (the "copy") | 75 µs | 39 µs | 41 µs |
  | read cutter, bytes → handle | 74 µs | 19 µs | 20 µs |
  | serialize block, handle → bytes | 63 µs | 41 µs | 43 µs |
  | `difference` FROM VALUES (2 reads + cut + serialize) | 3.99 ms | 3.09 ms | 3.13 ms |
  | `difference` FROM HANDLES (cut + serialize) | 3.28 ms | 3.04 ms | 3.09 ms |
  | `tessellate` hole FROM VALUE (read + mesh + weld; 8,803 B) | 1.24 ms | 990 µs | 981 µs |
  | `tessellate` hole FROM HANDLE (mesh + weld) | 996 µs | 920 µs | 907 µs |
  | CHAIN box → extrude → difference → tessellate, values (as shipped) | 4.56 ms | 4.62 ms | 4.54 ms |
  | CHAIN, handles kept (no re-reads — a cache's best case) | 4.38 ms | 4.33 ms | 4.30 ms |
  | CHAIN, values, on the rayon pool (24 logical threads), wall per part | 4.58 ms | 837 µs | 710 µs |

  Reading a block's bytes costs 41 µs against a 3.1 ms boolean (1.3 %);
  the whole chain pays 5 % for re-reading at every step (4.54 vs 4.30 ms
  per part; "prohibitive" would have been a copy costing more than the
  boolean itself). With no lock in the way the same chain runs 6.4×
  faster on the pool than serially (710 µs wall per part over 1,000
  parts).

  The cache is therefore not built; `Handle::from_value` is the one choke
  point it would wrap. It becomes both sound and worth building only if
  WP-C's glue makes the operations non-mutating — booleans in
  `SetNonDestructive(true)` mode (measure its cost: it copies the
  sub-shapes whose tolerances would change) and meshing preceded by
  `BRepTools::Clean` or run with `ForceFaceDeflection` — at which point
  rule 2 can be relaxed to "tessellate and booleans borrow" and a bounded
  per-hash handle map with per-handle locks slots in at that function.
- **Display tessellation (WP-B).** A `Solid` in a display set is drawn
  through `solid::tessellate` at `Deflection::display(&ProjectConfig)`:
  `linear = max(0.02 mm / unit.millimeters(), tol)`,
  `angular = max(0.1 rad, tol_angle)` — a PHYSICAL chord deviation of two
  hundredths of a millimetre (the same part looks the same in a mm, inch
  or metre document), floored at the coincidence tolerance because a
  finer tessellation is noise; 0.1 rad ≈ 5.7° gives ~63 facets per full
  turn, matching the 64 segments the analytic curves' display uses. The
  server caches the result by the Solid's VALUE hash plus the deflection
  (docs/12 §Display cache); the deflection never reaches the bytes.
  `Deflection::new` admits nothing finer than the kernel's own floors —
  `MIN_LINEAR_DEFLECTION` = 1e-7 (`Precision::Confusion()`) and
  `MIN_ANGULAR_DEFLECTION` = 1e-12 rad (`Precision::Angular()`) —
  because `BRepMesh_IncrementalMesh` throws `Standard_NumericError` for
  anything below them (the seam's test drives the raw glue below the
  floor to prove it is necessary, and the mesher at exactly the floor to
  prove it is sufficient); the refusal is a typed `BadParameter` naming
  the floor, and WP-C's `tessellate` node inherits it as its "Red when".
  The display formula cannot reach the floor (its minimum over every
  unit, 0.02 mm in a foot document, is 6.6e-5), which is why
  `Deflection::display` is infallible — pinned by a test over all five
  units at the finest tolerances `ProjectConfig` accepts.

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
