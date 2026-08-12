# Use cases

Cicada is built for one user profile first: a technical designer/maker who
thinks procedurally, is comfortable reading and lightly editing code, wants
an AI collaborator for new capability, and fabricates real objects at the
end. Generalize later; serve this person completely first.

## Primary use cases

### 1. Generative fabrication pipelines (the founding case)

The reference workload is the wall piece that motivated Cicada: a physics
field solve → Voronoi partition → ~1,500 leaning frustum parts → per-part
labels, pin holes, boolean carves → color-binned packing across a fleet of
five printers (two machine types, per-machine slicer policy) → Bambu 3MF
project files with per-object settings → CNC DXF for the baseboard →
manifests and print trackers. Characteristics that must be first-class:

- Thousands of index-aligned parts flowing through every stage; identity
  (part IDs) and alignment survive end to end.
- Long-running geometry stages (booleans, meshing) that must be cached,
  parallel, cancellable, and resumable.
- Fabrication artifacts as outputs: 3MF (vendor-flavored), DXF, CSV
  manifests, spreadsheets — with datum discipline (one source of truth per
  physical fact).
- Determinism: reruns byte-identical so diffs and reprints are meaningful.

### 2. Procedural part/product modeling ("simple traditional CAD")

Parametric B-rep parts defined in code — extrude, revolve, loft, sweep,
boolean, chamfer, modest fillets — with **STEP export** for interchange and
3MF/STL for printing. This is the OCCT B-rep tier: brackets, enclosures,
jigs, fixtures, adapters. Not targeted: Class-A surfacing, large
assemblies, PDM.

### 3. Hybrid prompt + script + node iteration

The working loop Cicada is really for:

- Describe a new capability ("extrude these pyramids to a flat triangular
  top oriented along the field") → the AI writes a typed function into the
  pipeline file → it appears as a node with a terse title, auto-derived
  ports, stored prompt, and an attached property test.
- Iterate by slider for tuned parameters, by direct code edit for formulas
  and constants, by prompt for refactors and new stages.
- Ask the AI questions *about the whole pipeline* ("why is this stage slow",
  "which nodes touch part IDs", "rename cells → regions everywhere") —
  possible only because the substrate is text.

### 4. Parameter exploration with live preview

Drag a slider; dirty stages recompute in the background with progress and
Esc-to-cancel; the viewer updates in place. Per-node preview toggles;
tap any wire to inspect its data (counts, bounds, samples) and see its
geometry. Click any object in the viewport and the node + element index
that produced it lights up.

### 5. Presentation renders

Fast instanced preview lives in Cicada; photorealism is delegated: export
the scene (geometry + ids + material bins + cameras) to Blender over USD
and drive headless Cycles renders. One command from pipeline to beauty
shot.

### 6. Animated and kinetic models

Kinetic mechanical art is a first-class workload. A looping `cycle`
param (0→1, adjustable period) drives mechanisms, orbits, and phase
offsets downstream; the loop is frame-quantized, so one pass warms
every cache and playback runs at display rate thereafter. An
unbounded `clock` covers open-ended motion. Play/pause/speed live in
a global transport bar; time is never ambient — the player feeds
values, so determinism, caching, and export reproducibility survive
animation (docs 08, 12, 13).

### 7. Later: light manual sketching

A 2D constrained-sketch → extrude workflow for the traditional-CAD tier.
The constraint solver is rented (planegcs or SolveSpace's libslvs); the
work is UI. Explicitly deferred until the procedural core is proven.

## Non-goals

- Grasshopper ecosystem parity (Kangaroo, the 500-component long tail).
- Building a B-rep kernel, or matching Parasolid-class fillets.
- Multi-user CAD/PDM, drawings/GD&T, simulation.
- End-to-end "AI generates the whole model from one prompt." The AI writes
  *reviewable typed stages*; the human owns the structure.
