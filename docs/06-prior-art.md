# Prior art: steal / avoid

| Project | Steal | Avoid |
|---|---|---|
| **Grasshopper** | Immediacy of sliders; tap-any-wire inspection; preview-in-viewport; the plugin culture's breadth of ambition | Tree semantics + implicit broadcasting (the #1 bug factory); single-threaded UI-thread solver with no cancel; graph-as-truth in an opaque binary (embedded code drifts — verified by direct audit of the wall project's .gh); wires silently dropped on param edits; gates that latch |
| **Houdini** | The gold standard of node+code hybrid (VEX inside nodes); proceduralism at production scale; caching discipline; "everything is geometry + attributes" — Cicada's named axes are attributes done type-safe | Its learning cliff; SOP/DOP sprawl; not CAD (no B-rep/STEP); price |
| **Blender Geometry Nodes** (+ Sverchok) | Fields concept; instancing performance; proof that node UIs can be mainstream | Untyped spaghetti at scale; no code layer (everything must be nodes — the exact opposite failure of code-only); no CAD output |
| **Enso / Luna** | The founding thesis (code↔graph duality) — and the scars that define Cicada's two-layer answer | Bidirectional mapping over an arbitrary language; per the postmortem reading: it collapses into "IDE with extra steps" |
| **Dynamo** | Confirmation that GH's model transplants (and re-grows the same pathologies) | Same tree/iteration semantics, same opacity |
| **OpenSCAD** | Code-first CSG conviction; tiny surface area; huge hobbyist adoption proves the audience exists | No live typed params UI; historically slow CGAL booleans; language is a dead end (Cicada uses a real host language) |
| **CadQuery / build123d** | The rent-OCCT strategy itself; fluent procedural B-rep APIs worth imitating in node form | No viewer/interaction layer (which is exactly the gap Cicada fills) |
| **nTopology** | Implicits as a product; contracts between stages; field-driven design | Closed, expensive, sim-industry-shaped |
| **Plasticity** | Existence proof: a solo developer shipped competitive CAD by *renting* a kernel (Parasolid) and pouring everything into interaction quality | Its direct-modeling scope (no parametric history) is the opposite tier |
| **Fornjot / truck** | Honest public documentation of why B-rep kernels take decades | Building the kernel |
| **ComfyUI** | How fast a node UI spreads when the underlying ops are AI-era exciting; graph sharing culture; caching UX | Untyped noodle chaos at scale; graph-as-truth (workflows are JSON blobs with the same drift problems) |
| **xarray / pandas** | Named axes as the cure for positional-index bugs; the mental model transfer is direct | — |
| **Observable / Jupyter** | Reactive re-execution bound to code cells; inspect-anything culture | Hidden execution-order state (Cicada's top layer is order-independent by construction) |

## The one-line synthesis

Houdini proved node+code at scale, Plasticity proved rent-the-kernel,
CadQuery proved open procedural B-rep, Manifold made mesh booleans a
solved problem, Enso documented the trap, and Grasshopper — by being
simultaneously indispensable and infuriating for twenty years — wrote the
requirements document. Cicada is the intersection.
