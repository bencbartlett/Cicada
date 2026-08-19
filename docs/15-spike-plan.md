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

## Stage-6 results (measured)

Dev machine, 2026-08-19, commit of the stage-6 slice: **Intel Core
i7-13700KF** (24 threads), **64 GB RAM**, NVMe SSD, Windows 11; the engine
built `--release`; the cache on the SSD in the user temp dir; every number
best-of-three where a spread is quoted. The wall slice is **1,200 parts**
(the frozen production layout — the wall shipped 1,200; "~1,500" in the
protocol was the candidate count before culling). The harness lives in
`corpus/measure/`.

| Criterion | Target | Measured | Verdict |
|---|---|---|---|
| **Carve speed** (`corpus/wall.cic --node carved`, cold cache) | full labeled carve < 10 s; warm < 100 ms | **cold 6.5 s** (solve wall; best 6.48 s), **warm 0.13 ms** | **PASS** — the wall's Rhino carve was ~30 min |
| **Live slider loop** — cheap cone (02-solids `size`) | p50 ≤ 16 ms, p95 ≤ 33 ms | server **p50 0.5 ms / p95 1.4 ms**, client round-trip p50 0.6 ms / p95 1.7 ms, 300/300 previews at 60 Hz | **PASS** |
| **Live slider loop** — the wall's `amps` (the field cone) | (as above) | server **p50 0.4 ms / p95 17.9 ms**, ~59 previews/s, no freeze | **PASS at p50**; the p95 is the one preview/s that lands mid-flight of the ~50 ms Python field solve — honest, not a freeze |
| **Live slider loop** — full-pipeline `deboss` (dirties labels → glyphs → carve) | full-pipeline slider degrades honestly (progress, no freeze) | ~4 s/generation, latest-wins supersession, continuous `running` statuses, longest server silence 174 ms | **PASS** (degrades honestly, as the protocol allows) |
| **Esc always works** (`esc.mjs`, deboss → carved, ×20) | time-to-idle p95 < 250 ms | client **p50 172 / p95 214 / max 219 ms**; server cancel→idle **p50 169 / p95 182 ms**, 0 missed | **PASS** |
| **Canvas round-trip** (`web/e2e/roundtrip.spec.ts`) | byte-exact writer fixtures; file edit → canvas < 500 ms | writer output byte-exact; file edit → canvas **30 / 100 / 111 / 99 / 104 ms** (5 trials) | **PASS** |
| **Output equivalence** (`normalize.py all`) | byte-identical modulo declared noise; every diff documented | overall **NOISE** — no unexplained difference | **PASS** |

Output-equivalence detail (`corpus/tools/normalize.py`, against
`corpus/golden/production/`): the two **pristine** production 3MFs (the H2
teal and sky-blue plate files) match entry-for-entry after normalization —
build-item translations within **3 µm**, bboxes within **10 µm**, volumes
within **0.09 %** — with triangle counts reported, not compared (Manifold's
tessellation vs Rhino's is the declared noise, and the deboss uses the
bundled DejaVu Sans Bold in place of Arial Black). The three **X1C** files
were re-saved by Bambu Studio (thumbnails added, XML rewritten, a dozen
objects nudged by hand); the normalizer detects the re-save and reports its
entry/XML/translation differences as declared noise while keeping object
names, plate membership, bbox and volume as hard checks — all of which
pass. The board **DXF** matches every entity of `board_postprocessed.dxf`
within **1 µm** across all five layers (OUTLINES, PINHOLES, BOARDCUT,
STOCK, TEXT). The `manifest.csv` matches all 1,137 rows to last-digit
rounding. Declared deviations, all documented in `corpus/README.md`: the
bundled font, 3MF zip timestamps fixed at 1980-01-01 (production stamped
the wall clock), Manifold vs Rhino tessellation, and the recovered-layout
sub-µm coordinate rounding.

What the measurements bought, recorded in the commits that made each number
(the scheduler stopped paying for work no consumer asked for): small value
blobs pack into one append-only file (cold 1,500-element fan-out 2.2 s →
0.14 s); the executor hydrates one output port instead of a node's whole
output vector (the wall-layout node's slider cone 250 ms → sub-ms); the
store reuses zstd contexts and batches a list's leaf appends; the cancel
check moved between elements (not only between chunks) and a cancelled
generation paints nothing (Esc ~246 ms at stage 5 → ~170 ms); and
`mesh_difference` uses Manifold's batch difference.

## Kill / pivot criteria

**Outcome: the gate passed — all five criteria met (results above).** None of the pivots below fired; they stay recorded as the tripwires they were.


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

Planning is closed (docs 01–16). The gate PASSED (stage-6 results above), so v0.1 begins:
OCCT-backed Solid, the full catalog, WASM script host, undo, git
panel, scrub caching, time transport — in whatever order the spike's
learnings argue for.
