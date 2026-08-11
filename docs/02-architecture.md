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
  node, order-independent, trivially parseable. This layer *is* the graph.

```python
# wall.ci.py — top layer (each binding is a node; this IS the canvas)
field   = solve_field(coil, samples, current=amps)      # Solve magnetic field
cells   = voronoi(seeds, board)                         # parts: Cell
frusta  = frustum(cells, field.dirs, heights)           # parts: Solid
labeled = deboss(frusta, ids(cells))                    # parts: Solid
carved  = carve(labeled, pin_cutters(cells, field))     # parts: Optional[Solid]
plates  = pack(carved, machines)                        # plates: Plate
```

- **Node bodies — arbitrary code.** Ordinary typed Python functions (in the
  same file or imported modules). The canvas never tries to render their
  internals; a node is its signature, title, and status.

- **Layout sidecar.** Node positions/colors/groups live in a sidecar block
  or `wall.ci.layout.json` that the differ ignores. Deleting it loses
  nothing but aesthetics; auto-layout regenerates it.

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
- **Broadcasting is always explicit**: `map`, `zip`, `flatten`, `graft`
  combinators, never inference. When you wire `[Curve]` into a
  `Curve`-taking node, the UI offers a one-click `map` lift — but the lift
  is recorded in the text, visible forever.
- **Nulls are typed and slot-preserving**: `parts: Optional[Solid]` is the
  honest type of a carve stage's output. Dropping a null (and silently
  shifting every later element onto the wrong part — the wall's nastiest
  bug class) is not something a combinator can do; removing elements
  requires an explicit `compact` that also returns the index map.
- The checker is **Cicada's own**, independent of the runtime language. It
  checks domain types (shapes, axes, geometry kinds, units), so it can be
  airtight while v1's runtime is Python.

## 3. The scheduler: why it feels fast

GH's slowness is not interpretation — geometry kernels and display dominate
— it is architecture: full-downstream re-solve, single-threaded, on the UI
thread, uncancellable. The Cicada runtime contract:

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

- Typed parameters → input ports; a returned dataclass → named output
  ports; the docstring's first line → the node title.
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
- **Tap any wire → inspect**: cached data summary (counts, bounds, samples)
  plus geometry preview in the viewer. GH's best feature, systematized —
  it's a caching feature, not a canvas feature.
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
- v1 graph view is read-only + parameter-editable. Structural edits happen
  in text (or via the AI); an editable canvas whose edits materialize as
  code diffs is deferred until the read-only view proves insufficient.

## 6. The AI integration model

The AI is a collaborator over the whole program, not a node-inserter:

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
