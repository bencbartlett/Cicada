# The vertical-slice spike: task plan

The empirical gate from doc 05, decomposed into buildable stages.
Target: two to three weeks of one human directing agents. Every
subsystem ships its **thinnest honest slice** — real code on the real
architecture, nothing mocked that the criteria depend on. If the spike
hits the numbers, the "build the whole thing" question is answered; if
not, the failure names the wrong assumption. Cheap either way.

## Scope

**In**: core values + hashing; `#[node]`/`Ports` macros + registry +
catalog generation; dialect parser/writer subset + checker-lite;
scheduler-lite (generations, disk memoization, parallel fan-out,
cancellation, latest-wins previews); ~30 mesh-tier stdlib nodes;
Python script host (the wall's numpy/scipy stages + exporters run as
script nodes); `cicada serve` + protocol + React Flow canvas + three.js
instanced viewport with picking; the wall slice end-to-end.

**Out, explicitly** (all v0.1+): OCCT/B-rep backing, WASM script host,
undo/redo, git panel, scrub caching, time transport, AI layer,
refinement conversions beyond `As Closed`/`As Watertight`, sidecar
beyond positions (+ the preview toggle, added in stage 5), auto-layout
beyond grid-append (stage 5 ships "layer by dependency depth, stack in
definition order" — deterministic and a few lines; anything smarter
stays out).

**One honest shim**: spike `Extrude`/`Box`/`Sphere` are mesh-backed
under their v0.1 names (Solid is B-rep-backed from v0.1; the spike's
workload is mesh-destined, so nothing in the criteria needs OCCT).
The frusta are analytic mesh constructions, as on the wall.

## Stages

### Stage 0 — scaffold (the standards land before the code)

Workspace per doc 14 (nine crates, `web/`, `corpus/`), CI skeleton
(fmt, clippy pedantic, tests, WASM/web checks stubbed), root
`CLAUDE.md`/`AGENTS.md`, `.claude/skills/` (`verify-change`,
`add-stdlib-node` first), `docs/generated/CATALOG.md` generation
wired even while nearly empty.
**Done when**: a fresh clone builds green in CI; an agent can read
the repo's own manual and add a stub node end to end.

### Stage 1 — values and registry (`cicada-core`, `cicada-macros`)

Value model (scalars, Point/Vector/Plane/Xform, Domain, Color, Text,
IndexMap), blake3 hash-at-construction, interning, Merkle lists +
axes + Optional slots, `ProjectConfig`; `#[node]` + `#[derive(Ports)]`
reflecting NodeSpecs into the registry; catalog JSON + CATALOG.md.
**Done when**: determinism tests pass (same value → same hash across
runs/platforms); a `#[node]` function round-trips into the catalog
with ports, defaults, and doc lines intact.

### Stage 2 — dialect and checker-lite (`cicada-lang`)

Parser for the spike subset (pragma, comments, bindings, kwargs-only
calls, literals, `each()`, expression RHS, multi-output unpack, port
selection); the minimal-edit writer (every doc 10 gesture the spike
canvas needs: place, wire, lift, set-param, delete, rename); checker
over the kind lattice + lists + `each()` pairing (strict zip, counts
in errors) + the two refinements; diagnostics in the doc 11 JSON
shape.
**Done when**: golden round-trip corpus is byte-identical; every
gesture has a fixture; wrong wires produce the specified diagnostics
(snapshot-tested); a broken statement reds one node, not the file.

### Stage 3 — scheduler-lite (`cicada-sched`)

Generations with supersession; dirty-cone computation; rayon
wavefront + `each()` element fan-out with ~10–50 ms chunks; memo
table + content-addressed value store on disk (user cache dir);
cancellation tokens; the latest-wins preview path; cost sampling
(recorded, even if the estimator stays naive).
**Done when**: virtual-time fake-node tests prove cache hits, exact
dirty cones, warm reopen computing nothing, and cancel-to-idle
< 100 ms; a synthetic 1,500-element map saturates cores.

### Stage 4 — geometry, stdlib, Python host (`cicada-geom`, `cicada-stdlib`, `cicada-script`)

The ~30 S-tier nodes (docs/08): params (slider, literals, panel),
sequences (series, random), maths (+ Remap, Expression), lists (item,
length, map/zip), point/vector/plane constructors, curves (line,
polyline, circle, rectangle, divide, As Closed), mesh tier (extrude,
box, sphere, As Watertight, **Mesh Boolean via manifold3d**),
transforms (move, rotate, scale, orient, linear array), Voronoi
(spade), preview/text-tag display nodes. Python worker pool with
MessagePack marshalling; the wall's field solver runs as the first
script node.
**Done when**: every node has table + property + determinism-hash
tests; the standalone carve benchmark (1,500 labeled frusta ∖
cutters) lands in seconds on the dev machine; kill-the-worker
cancellation works. **Verify here**: manifold3d binding quality and
Manifold's precision model (fallback: thin C-API shim, budget one
day).

### Stage 5 — server and app (`cicada-server`, `cicada-cli`, `web/`)

axum serve (localhost + token, embedded SPA in release, Vite proxy in
dev); protocol control plane + binary mesh/instance frames with
generation tags and pick IDs; React Flow canvas (search-to-place,
typed ports, wire drag with live compatibility, lift chips, red
wires, sliders on canvas); three.js viewport (instanced draws,
ID-buffer picking → node + element highlight); param-drag preview
path wired end to end; `/debug/state` + `/debug/screenshot`. Layout
and interaction contracts per doc 16 (spike ships the canvas-focus
layout, inspector, and keyboard map's core rows).
**Done when**: Playwright smoke passes headlessly (serve → place →
wire → drag → screenshot asserts geometry changed); backward picking
demo works; agents can verify UI changes without a human.

### Stage 6 — the wall slice, measured

Port the stretch: field solve (Python) → Voronoi → frustum build →
label deboss → pin-hole carve (Manifold) → pack (Python) → Bambu 3MF
+ DXF exporters (Python, ported wall writers). Run the measurement
protocol below; write the numbers into this document and the README.
**Done when**: all five criteria have measured values, pass or fail.

Stages 4 and 5 touch disjoint crates and run in parallel once 3
lands. Rough calendar: week 1 = stages 0–3; week 2 = 4 + 5; week 3 =
6 + slack for what the measurements surface.

## Measurement protocol

| Criterion | How measured | Target |
|---|---|---|
| Carve speed | `cicada run corpus/wall.cic --node carved --time`, cold cache, dev desktop | Full ~1,500-part labeled carve **< 10 s** (wall baseline: ~30 min in Rhino); warm rerun < 100 ms |
| Live slider loop | Drag the demo cone's slider 5 s; tracing spans report preview-generation latency | p50 ≤ 16 ms (60 fps) on the cheap cone; p95 ≤ 33 ms; the full-pipeline slider degrades honestly (progress, no freeze) |
| Esc always works | Scripted cancel injected mid-carve ×20 (`corpus/measure/esc.mjs`; server-side `timings[].cancel_to_idle_ms` from the cancel call to the loop idle, plus the client's poll-observed time-to-idle) | Time-to-idle p95 **< 250 ms**; UI thread never blocks |
| Canvas round-trip | Playwright: place + wire → assert file text; edit file → assert canvas | Byte-exact writer fixtures; canvas reflects file edits < 500 ms |
| Output equivalence | Hash-compare 3MF/DXF against production wall exports through a normalizer (strips timestamps/UUIDs) | Byte-identical modulo declared noise; every legit diff documented |

Hardware note: numbers are recorded with the dev machine's spec
attached; targets assume a mid-to-high-end desktop (doc 12).

## Kill / pivot criteria

- Carve not dramatically under the Rhino baseline → the mesh-tier
  assumption is wrong somewhere (binding overhead, mesh quality);
  investigate before building more.
- Slider loop can't feel live even with warm caches → scheduler
  architecture problem; fix before investing in UI polish.
- manifold3d bindings inadequate → C-API shim (one day), not a
  redesign.
- React Flow can't hold the canvas feel at wall scale (~350 nodes) →
  evaluate custom canvas before v0.1 commits deeper.

## After the spike

Planning is closed (docs 01–16). If the gate passes, v0.1 begins:
OCCT-backed Solid, the full catalog, WASM script host, undo, git
panel, scrub caching, time transport — in whatever order the spike's
learnings argue for.
