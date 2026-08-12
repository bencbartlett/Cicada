# Roadmap

Scope discipline is the whole game: the semantics are mostly specified (file
format, types, scheduler, script nodes); the unbounded tail is editor
polish, and it stays deferred until evidence demands it.

## The vertical-slice spike (empirical gate)

Two to three weeks, in the shipping stack: a Rust core skeleton (core
types, `#[node]` registry, scheduler-lite with caching + cancellation),
~20 stdlib nodes, and a browser UI served by the local engine
(`cicada serve`) — React Flow canvas (place, wire, slider) and a
three.js instanced viewport with picking (web-first, doc 04). Port the wall pipeline's
hardest stretch — field solve → Voronoi → frustum build → pin-hole
carve → pack — running the existing numpy/scipy wall scripts as Python
script nodes and **Manifold** (Rust) for the carve. The wall scripts
were architected to run outside Rhino and have offline test suites; they
are the library.

**Success criteria** (measured, not vibed):

- Full-scale carve (~1,500 parts with debossed labels) in seconds, not the
  half-hour Rhino ordeal.
- Slider → recompute → viewer loop fast enough to feel live.
- Esc cancels any solve, always.
- Canvas round-trip: place and wire nodes by drag-and-drop; the dialect
  file updates; edit the file; the canvas updates.
- Output equivalence: 3MF/DXF byte-comparable (modulo format noise) to the
  production wall exports.

If the spike hits these, the "should I build the whole thing" question is
answered empirically. If not, the failure mode tells us which assumption
was wrong — cheap either way.

## v0.1 — the core is real

- Dataflow dialect parser + **shape/axis checker** (red wires, explicit
  combinators, Optional slots, one-click recorded lifts).
- **Scheduler**: content-hash caching, minimal recompute, parallel
  execution, disk memoization, cancellation, progress/ETA, profiler.
- **User code**: expression nodes (typed IR); script nodes — Rust→WASM
  by default, Python 3 subprocess available; signature→ports, re-parse
  on save, stale-wire type errors, prompt provenance, `@contract`
  property tests.
- **Git in the UI**: status strip with per-node change markers,
  node-level visual graph diff, commit from the app, per-node history
  (doc 10).
- **Stdlib**: the full docs/08 catalog (mesh tier; B-rep tier lands in
  v0.2).
- **Editable canvas**: GH-style placement/wiring/param editing
  materializing as dialect text edits; wire inspection, per-node preview
  toggles, **backward picking**.
- Fabrication exporters ported: Bambu 3MF, DXF, manifests.
- Test corpus: the full wall pipeline reproduced end-to-end under CI.

## v0.2 — the CAD tier + beauty

- OCCT backend behind the seam (`opencascade-rs`; build123d as API prior
  art): procedural B-rep nodes, STEP in/out, tessellation node.
- **Blender bridge**: USD export with IDs/materials, template .blend,
  headless `cicada render` (Cycles), camera bookmarks.
- fidget backend for implicit blends/lattices (fillet-as-smooth-min).
- .gh migration importer (GH_IO-based) for recovering old definitions.
- Desktop app: a thin Tauri wrapper bundling the local engine + web UI.
- AI layer v1: prompt→node (Rust by default) with diff review,
  whole-pipeline refactors, "why is this slow" over the profiler.

## Later / on evidence only

- **2D sketcher** on planegcs/libslvs — when the CAD tier earns it.
- **Native wgpu viewer** — only if the webview viewport hits a real
  ceiling on real scenes.
- Shareable interactive scenes (the web viewport travels well).
- Live Blender link.

## Risks, named

- **Editor-polish tail**: undo, selection ergonomics, wire management —
  the place node editors go to die, and now a real risk (the canvas is
  editable in v1). Mitigation: rent the substrate (React Flow), hold the
  line at the GH-familiar checklist, and keep text editing as the escape
  hatch for anything the canvas can't do yet.
- **Scope creep toward GH parity**: the 500-component long tail. Mitigation:
  the use-case doc's non-goals list; every new node must serve the corpus
  or a real project.
- **Kernel-ceiling surprises**: OCCT failing on real B-rep work. Mitigation:
  the seam — route the failing op to Rhino.Compute and file it as data.
- **AI-node sprawl**: dozens of half-trusted generated stages. Mitigation:
  provenance + mandatory contracts + the human-owned top layer.
