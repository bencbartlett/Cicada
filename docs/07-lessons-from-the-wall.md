# Lessons from the wall

The wall piece — ~1,500 Voronoi frustum parts rendering a magnetic field,
five filament colors across a five-printer fleet, CNC'd baseboard —
was built end-to-end in Grasshopper + ten Python script components. It is
Cicada's requirements document: every failure class observed in production,
mapped to the mechanism that makes it unrepresentable. (The general
patterns live in the `grasshopper-pipelines` skill / GRASSHOPPER_LESSONS.md
in the wall repo; this file is the Cicada-specific mapping.)

| # | What happened in Grasshopper | Root cause | Cicada mechanism |
|---|---|---|---|
| 1 | One solve silently ran a full export **66 times** (once per item); hour-long "infinite loops"; wrong-but-plausible output files | Item-access input fed a list → whole-script re-execution per item; no native strict mode | Shape-typed wires: `[T]` into a `T` port is a **compile error** with a one-click, text-recorded `map` lift. The iteration model does not exist |
| 2 | Coil exports carried **wrong geometry on right names**: a collector dropped None slots and every later part shifted one index | Nulls are legitimate data in index-matched lists; generic flatteners drop them | `Optional` element types; slot-preserving semantics in every combinator; element removal only via explicit `compact` returning an index map |
| 3 | Flatten/graft dances; loft pairing bugs; branch-order pairing broken by missing branches | Untyped tree path-matching deciding semantics implicitly | Named axes (`parts: Curve`); explicit `zip over parts`; empty axis elements exist and are typed |
| 4 | 5-minute freezes → force-quit; Windows EcoQoS throttling the frozen solve; Esc unreachable | Single-threaded UI-thread solver, uncancellable, full-downstream re-solve | Background scheduler: parallel, minimal-recompute, disk-memoized, **cancellation everywhere**, progress+ETA as runtime services |
| 5 | Production .gh audited (via GH_IO): the packer component ran code **74 lines behind disk**; the colorizer existed *only* inside the binary, matching no commit | Graph-as-truth; code embedded (base64) in an opaque blob; three places code can live | Two-layer text file; code on disk is the only place logic lives; the graph is derived, so drift is unrepresentable; everything diffs under git |
| 6 | Boolean carve took ~half an hour and was the fragility epicenter; curved NURBS cutters froze for 30 minutes | Kernel mismatch: NURBS booleans for FDM-bound geometry | Mesh-native default: Manifold booleans (watertight, parallel, seconds); B-rep only where B-rep semantics are needed, behind the seam |
| 7 | Saving with Export gates latched ON meant reopening the file relaunched the full production run | Stateful toggles double as run controls | Runs are explicit actions; caching makes re-runs cheap; nothing auto-fires on open |
| 8 | Killing Rhino lost the entire in-memory solve; partial progress survived only because scripts hand-wrote files incrementally | No persistent memoization | Disk-persistent per-node cache; crash costs one node's work |
| 9 | Progress was invisible or misleading ("plate 15/16: part 2/14" resetting); throttling misdiagnosed for hours | Progress hand-rolled per script; no global cost model | Scheduler-owned global progress, ETA, and per-node compute+display profiling |
| 10 | Text glyphs and DimStyle fights; per-entity size overrides ignored | Doc-global styles leaking into procedural geometry | ttf-parser/rustybuzz outlines; no ambient document state — everything is a typed input |
| 11 | The layout froze late and every derived ordering (IDs, bins, packing) had to regenerate together or drift | Derived orderings renumber when the part set changes | Determinism contract + content hashing: same inputs, byte-identical outputs; stale derivations are cache-invalidated automatically, not by discipline |
| 12 | Offline exec-shim test suites caught nearly every bug before GH ran the code — the single best architectural decision | Testability was retrofitted against the host's grain | Productized: `@contract` property tests on nodes, evaluated on live data; the corpus runs under CI |
| 13 | Debug-as-output + loud refusal (withheld DXF without a datum) repeatedly prevented fabrication disasters | Silent fallbacks breed mysteries | First-class node status; refusal semantics in the type system (a node without required inputs is red, not guessing) |
| 14 | One datum rule: deriving everything from `BoardOutline` after redundant width/height inputs diverged | Redundant specifications of one fact | Typed project constants; single-source derivations are the idiom the dialect encourages |
| 15 | What GH did **right** and must survive: slider immediacy, tap-a-wire inspection, preview-in-viewport, visual sanity checks catching what tests missed (the slicer screenshot that exposed the every-other-object bug) | — | Params panel, wire inspectors, per-node preview, backward picking — kept, systematized, and made honest by generation-from-code |

The through-line: almost nothing on this list is a geometry problem. It is
all representation, scheduling, and semantics — which is exactly why a
small tool with the right architecture can beat a twenty-year incumbent
for this class of work.
