# Architecture

Four load-bearing ideas: a two-layer file format, shape-typed wires, a
cached preemptible scheduler, and signature-derived script nodes. Everything
else is a view over those.

## 1. The two-layer file format

**The lesson from Enso/Luna** (years spent on bidirectional code↔graph):
do not make the mapping bidirectional over an arbitrary language. If the
top level permits general control flow, the graph view stops being faithful
and the tool collapses into "IDE with extra steps."

A Cicada pipeline file has two layers:

- **Top layer — the dataflow dialect.** Straight-line bindings, one per
  node, order-independent, trivially parseable. This layer *is* the
  graph. (Full grammar and the canvas round-trip contract: doc 10.)

```python
# wall.cic — top layer (each binding is a node; this IS the canvas)
amps = slider(value=12.0, min=0.0, max=30.0)
field = solve_field(coil=coil, samples=samples, current=amps)
cells: parts = voronoi(seeds=seeds, boundary=board)
frusta = frustum(profile=each(cells), direction=each(field.dirs), height=each(heights))
labels = ids(cell=each(cells))
labeled = deboss(solid=each(frusta), text=each(labels))   # zip over parts
cutters = pin_cutters(cell=each(cells), field=field)
carved = carve(solid=each(labeled), cutter=each(cutters))  # parts: Solid?
plates = pack(parts=carved, machines=machines)
```

- **Node bodies — arbitrary code.** Stdlib nodes are typed Rust functions
  compiled into the engine. User code enters at two tiers (§4):
  expressions live inline in the pipeline file; script nodes (Rust→WASM
  by default; Python 3 available) live in sibling source files the
  binding references. The canvas never renders their internals; a node
  is its signature, title, and status.

- **Layout sidecar.** Node positions/colors/groups live in
  `wall.cic.layout.json`, which the differ ignores. Deleting it loses
  nothing but aesthetics; auto-layout regenerates it (doc 10).

Consequences: the file is git-diffable, the graph can never drift from the
code (it is *derived* from it), and there is exactly one place logic lives —
the wall project's .gh audit (production code 74 lines stale inside the
blob; one module existing nowhere else) is unrepresentable.

## 2. The type system: shapes and named axes

Grasshopper's data trees are an untyped implicit-broadcasting system; the
bugs that matter are **shape errors**, and GH resolves them silently
(longest-list matching, per-item re-execution). Cicada types the shapes:

- `Curve`, `[Curve]`, `[[Curve]]` are distinct types. A node whose input is
  `[Solid]` shown a `[[Solid]]` is a **red wire before anything solves** —
  the wall project's worst freeze, converted to a compile error.
- **Named axes** (xarray-style): `parts: Curve` means "one Curve per part."
  The wall pipeline's lists were all implicitly indexed by axes named
  `parts`, `plates`, `colors`; naming them makes cross-axis mistakes
  unrepresentable and turns "loft base to cap" into literally
  `zip over parts(base, cap)`.
- **Broadcasting is always explicit**: `map`, strict `zip`, `cross`,
  `flatten`, `nest`, `squeeze`, `transpose`, `compact` — never inference
  (GH's tree vocabulary is retired; doc 09). Wiring `[Curve]` into a
  `Curve`-taking node offers a one-click `map` lift, recorded in the
  text and worn by the port as a persistent iteration badge (`map`,
  `×2` when nested). Scalars close over a map (one `motion` against
  1,500 geometries); two lists pair only by `zip`, and length mismatch
  is an error with opt-in policies (`pad_last`, `cycle`, `truncate`).
- **Incompatible wires cannot be drawn**: during drag, incompatible
  ports are blocked with a reason; liftable ones connect only through
  the recorded adapter chip. The wrong wire never exists, even
  transiently.
- **Nulls are typed and slot-preserving**: `parts: Optional[Solid]` is the
  honest type of a carve stage's output. Dropping a null (and silently
  shifting every later element onto the wrong part — the wall's nastiest
  bug class) is not something a combinator can do; removing elements
  requires an explicit `compact` that also returns the index map.
- The checker is **Cicada's own**, independent of implementation language
  (Rust engine, Python script nodes). The kind hierarchy
  (`Circle <: Curve <: Geometry`) is a subtype lattice inside the
  checker, not host-language inheritance — so generics are
  **kind-preserving**: `Move: (T: Transformable, Vector) → T` returns
  `[Circle]` for `[Circle]`, not an anonymous Geometry.
- **Refinements, not subclasses, for data-dependent properties**:
  closedness and planarity are properties of the data, so `Closed<Curve>`
  and `Planar<Curve>` are wrapper types entered only through explicit
  checked conversions — red with the offending element IDs on failure.
  Total upcasts (Circle → Curve) are implicit and free.

## 3. The scheduler: why it feels fast

GH's slowness is not interpretation — geometry kernels and display dominate
— it is architecture: full-downstream re-solve, single-threaded, on the UI
thread, uncancellable. The Cicada runtime contract (internals: doc 12):

- **Content-hash caching per node**: inputs hashed (geometry by content,
  params by value, code by AST hash); unchanged nodes never recompute.
- **Minimal recompute**: edits dirty exactly the downstream cone.
- **Parallel topological execution** across independent branches and across
  elements of mapped axes (the wall's chunked-parallel carve, as a runtime
  service instead of hand-rolled TPL).
- **Disk-persistent memoization**: reopening a project restores every cache;
  a crash costs nothing (the wall lesson: killing Rhino lost the whole
  in-memory solve).
- **Cancellation everywhere**: Esc preempts any solve at the next safe
  point; the UI thread never blocks. Progress + ETA are runtime services
  fed by per-node cost models (measured, cached).
- **Determinism as a contract**: stable sorts, explicit tie-breaks, no
  ambient randomness; identical inputs → byte-identical outputs, so diffs,
  reprints, and cache hits are always meaningful.

## 4. Script nodes: the signature is the ports

Stdlib nodes get their ports from the struct-in/struct-out node ABI
(`#[node]` + `#[derive(Ports)]`, docs/08) at compile time — input-struct
fields are the input ports, and the same field names run end to end:
Rust call sites, JSON catalog, canvas labels, dialect kwargs. User code
gets ports from parsing at save. Everything registers into one typed
catalog (serialized as JSON) that drives the palette, the checker, and
the AI.

User code enters at two tiers, ports auto-derived in both:

1. **Expression nodes** — one formula in language-neutral math syntax
   (`z = x^2 + y^2`, `^` is power). Free variables become input ports;
   the assignment target names the output. Compiled to a typed IR
   evaluated natively.
2. **Script nodes** — real code in sibling source files. **Rust by
   default — including AI-generated nodes** — compiled to sandboxed
   WASM (wasmtime): near-native speed, a crash costs one node, epoch
   preemption gives hard cancellation. A **Python 3 script node** (full
   CPython in a subprocess pool: numpy/scipy/rhino3dm) exists like
   Grasshopper's, with typed marshalling at the boundary — an option at
   the edges, never the core representation.

- Input-struct fields → input ports; output-struct fields → named
  output ports (bare return = single `out` port; Python script nodes
  mirror the ABI with dataclasses); the doc comment's first line → the
  node title.
- Re-parse on save (hot reload). When a signature changes, stale wires
  become **type errors, not silent deletions** — GH dropping wires on
  param edits was one of its worst behaviors.
- **AI provenance**: an AI-written node stores its generating prompt
  alongside the code, so refinement has history ("same intent, but make the
  caps triangular").
- **Property tests as contracts**: any node may carry a test
  (`@contract`) evaluated against current data on solve; failure renders
  the node red with the counterexample. This is the mechanism that turns
  vibe-coding into continuously verified engineering — and it is the
  productized form of the wall project's offline test suites, which caught
  nearly every bug before the canvas ever ran the code.
- **Debug is a first-class output**: every node exposes status — counts,
  parameter echoes, warnings with offending IDs and type names. Loud
  refusal beats silent fallback (a fabrication artifact with a wrong datum
  is worse than none).

## 5. The interaction model

The graph view is generated, live, and honest:

- ~10 nodes for a 10-stage pipeline (the wall's canvas needed 358 objects
  to say the same thing).
- Click a node → contract, parameters, runtime stats, warnings, prompt
  provenance, test status.
- **Tap any wire → inspect**: cached data summary (counts, bounds,
  samples) plus geometry preview in the viewer; on lifted wires, the
  exact pairing (what mapped/zipped over what, with counts). GH's best
  feature, systematized — it's a caching feature, not a canvas feature.
- **Backward picking**: click an object in the viewport → the producing
  node and element index light up (with the named-axis index: `parts[412]`,
  id `C12`). Code-first tools all lack this; it is a launch requirement.
- Parameters panel auto-generated from typed params with ranges: sliders,
  toggles, enums. Direct manipulation survives; prose is for new
  capability, not iteration.
- Display is a **first-class edge with visible cost** — preview toggles
  per node, instanced rendering for thousands-of-objects scenes, and the
  profiler shows display cost next to compute cost (the wall lesson:
  display was half the pain).
- The canvas is **fully editable, GH-style**: search-to-place, drag wires
  with live type-compatibility highlighting, sliders and panels on
  canvas, groups. Every canvas edit materializes as a text edit to the
  dialect layer — safe because the dialect is restricted (the Enso trap
  requires an arbitrary language). Canvas and text editor are two views
  over one file; neither can drift.

## 6. The AI integration model

The AI is a collaborator over the whole program, not a node-inserter
(the concrete editing loop, read tools, refactor primitives, and
permission tiers are doc 11):

- Generate: prompt → new typed function + node + test, diffed for review.
- Refactor: rename axes, split stages, change signatures — whole-file
  operations a graph substrate can't support.
- Explain: "what feeds this", "why is this slow" (reads the profiler),
  "what breaks if I change this signature" (reads the checker).
- Verify: proposes property tests; contracts run continuously.

Every AI edit is a text diff under version control. The human's mid-level
understanding is maintained by the artifacts that actually worked on the
wall project: typed contracts, node titles, Debug outputs, the decisions
ledger — now generated and enforced instead of hand-maintained.
