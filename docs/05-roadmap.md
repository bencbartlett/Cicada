# Roadmap

Releases are **Broods**, numbered like real cicada cohorts. Scope
discipline is the whole game: the semantics are mostly specified (file
format, types, scheduler, script nodes); the unbounded tail is editor
polish, and it stays deferred until evidence demands it.

## Brood 0 — the weekend spike (empirical gate)

Port the wall pipeline's hardest stretch — field solve → Voronoi →
frustum build → pin-hole carve → pack — to the Cicada stack:
numpy/scipy for field + cells, analytic mesh construction for frusta,
**Manifold** for the carve, **polyscope** for the viewer, hot-reload on
save. Use the existing wall scripts as the library (they were architected
to run outside Rhino and have offline test suites).

**Success criteria** (measured, not vibed):

- Full-scale carve (~1,500 parts with debossed labels) in seconds, not the
  half-hour Rhino ordeal.
- Slider → recompute → viewer loop fast enough to feel live.
- Esc cancels any solve, always.
- Output equivalence: 3MF/DXF byte-comparable (modulo format noise) to the
  production wall exports.

If the spike hits these, the "should I build the whole thing" question is
answered empirically. If not, the failure mode tells us which assumption
was wrong — cheap either way.

## Brood I — the core is real

- Dataflow dialect parser + **shape/axis checker** (red wires, explicit
  combinators, Optional slots, one-click recorded lifts).
- **Scheduler**: content-hash caching, minimal recompute, parallel
  execution, disk memoization, cancellation, progress/ETA, profiler.
- **Script nodes**: signature→ports, docstring titles, re-parse on save,
  stale-wire type errors, prompt provenance, `@contract` property tests.
- Generated **graph view** (read-only + parameter panel), wire inspection,
  per-node preview toggles, **backward picking**.
- Fabrication exporters ported: Bambu 3MF, DXF, manifests.
- Test corpus: the full wall pipeline reproduced end-to-end under CI.

## Brood II — the CAD tier + beauty

- OCCT/build123d backend behind the seam: procedural B-rep nodes, STEP
  in/out, tessellation node.
- **Blender bridge**: USD export with IDs/materials, template .blend,
  headless `cicada render` (Cycles), camera bookmarks.
- libfive backend for implicit blends/lattices (fillet-as-smooth-min).
- .gh migration importer (GH_IO-based) for recovering old definitions.
- AI layer v1: prompt→node with diff review, whole-pipeline refactors,
  "why is this slow" over the profiler.

## Later / on evidence only

- **Editable canvas** (edits materialize as code diffs) — only if the
  read-only view demonstrably chafes after a real project.
- **2D sketcher** on planegcs/libslvs — when the CAD tier earns it.
- **Rust scheduler/checker port** — only if profiling says Python is the
  bottleneck (for ~2,000-part pipelines it probably never will be; the
  time is in kernels and display, which are already native).
- Web viewer / shareable interactive scenes.
- Live Blender link.

## Risks, named

- **Editor-polish tail**: undo, selection ergonomics, wire management —
  the place node editors go to die. Mitigation: read-only canvas + text
  editing keeps v1 out of that swamp entirely.
- **Scope creep toward GH parity**: the 500-component long tail. Mitigation:
  the use-case doc's non-goals list; every new node must serve the corpus
  or a real project.
- **Kernel-ceiling surprises**: OCCT failing on real B-rep work. Mitigation:
  the seam — route the failing op to Rhino.Compute and file it as data.
- **AI-node sprawl**: dozens of half-trusted generated stages. Mitigation:
  provenance + mandatory contracts + the human-owned top layer.
