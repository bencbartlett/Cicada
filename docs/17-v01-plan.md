# v0.1 plan — the work order after the spike

The spike (doc 15) passed its gate on 2026-08-19. This document is the
v0.1 counterpart: the items, their order, the work packages each splits
into, what "done" means for each, and the process rules that keep the
work resumable across sessions. The order and the design choices were
decided with Ben on 2026-08-19 (DECISIONS.md row of that date); the
rows it cites are binding, this file is the schedule.

## Scope

Seven items from doc 05 §v0.1, in this order, plus a catalog track that
runs in parallel from day 1:

| # | Item | Track | Size | Status |
|---|---|---|---|---|
| 0 | Fold `corpus/` into `examples/wall/` + `tools/`; this plan | foreground | hours | **done** 2026-08-20 (every nightly step passes locally from the new paths; the first Nightly run itself is pending the push) |
| 1 | Undo/redo — snapshot op log + atomic `batch`/`apply_text` path; riders `#off`, Backspace-no-delete | foreground (server/web) | days | **done** 2026-08-20 (merged) |
| 2 | Git panel slice 1 — status strip, per-node change markers, commit, revert-to-HEAD | foreground (server/web) | ~1 week | pending |
| P | OCCT probe — prebuilt 7.8.x build/link on win/mac/linux, determinism, timings, license, CI shape | parallel worktree | hours, cap 1 day | **done** 2026-08-20 — GREEN on win-64 with one rename patch, byte-deterministic, ~3 ms/boolean; Linux/macOS measured by item 3 WP-A's CI job; memo `docs/probes/occt-2026-08.md` |
| 3 | OCCT-backed Solid — the `Solid` kind, primitives/extrude/loft/revolve/sweep, booleans, `tessellate`, STEP; `mesh_*` renames in the same commit | main geometry track from week 3 | weeks | **unblocked** (probe GREEN) — WP-A next: own fork with the recorded patches, `occt` feature, `tools/fetch_occt.py`, the per-OS CI job |
| 3b | Scheduler foundations — per-solve cancel handle, `volatile`, idle-class hypothetical solve — plus compute-on-release | parallel (sched/server) | ~1 week | pending |
| 4 | Time transport — Cycle thin slice + orbit example; Clock via `volatile` | foreground | ~1 week | pending |
| 5 | Scrub caching — bounded-position sliders only, toggleable, buffer bar | foreground | 1–2 weeks | pending |
| 6 | WASM script host — load precompiled guests, epoch cancellation, `cicada-guest` SDK | last | weeks | pending |
| C | Catalog — one-node-per-file restructure, node-format conformance test, then the docs/08 S+1 list in tranches; `cicada mcp` | parallel worktrees, continuous | continuous | **C0 done** (2026-08-20); **C1 done** (2026-08-20: 48 nodes — lists, maths tail, sequences; the diagnostics name real nodes and a test keeps it so; `compact` satisfiable at check time; `examples/06-lists.cic`); `cicada mcp` not started; C2+ pending |

Out of v0.1 (unchanged from doc 05): fillets/chamfers and B-rep
maturity, the Blender bridge, fidget, the .gh importer, Tauri, the AI
layer (the `batch` path is its landing pad), the 2D sketcher.

## Process rules (how this stays resumable)

- **The repo is the memory.** This file carries status; AGENTS.md
  carries the one-paragraph "what is live"; DECISIONS.md carries the
  choices. A new session reads those three and loses nothing.
- **Work packages fit one subagent context.** The orchestrating session
  freezes a contract (types, ports, protocol shape), subagents build the
  packages in worktrees with private `CARGO_TARGET_DIR`s, the
  orchestrator merges. It holds summaries, never the work.
- **Every package ends in a commit.** Uncommitted work never spans a
  session boundary. Probes write their memo to a file as they go.
- **Review before merge**: the 5-lens + critic adversarial review with
  reproduce-or-refute verification (the pattern that shipped stage 6).
  The most valuable lens is "can a check report a false PASS".
- Commit conventions, doc-update rule, catalog regeneration, and the
  definition of done are AGENTS.md's; they apply unchanged.

## Item 0 — the corpus move (hours)

`corpus/` becomes `examples/wall/` (pipeline, scripts, inputs, golden
production references, README, the wall-only layout tools) and `tools/`
(the measurement harness, `normalize.py` and its tests — engine-wide).
Nightly, CI's per-PR `corpus-offline` job, docs, AGENTS.md, the skills
and `.gitignore` paths follow in the same commit. **Done when**: `git mv`
history preserved; the nightly corpus job runs from the new paths and
passes; `python -m unittest discover -s tools -p "test_*.py"` passes;
`examples/wall` opens in the app; no reference to `corpus/` remains
outside history.

Landed in 60656b5 + the review follow-up. The generated data's
provenance stamps (`layout.json`, the extraction / seed-recovery reports,
`plates_summary.json`) were regenerated through the blessed
extract → recover → extract path so they name the tools where they now
live — the offline suite asserts every stamp points at a file in the
repo, and that the frozen inputs are present and are the 1,200-part
production wall. The nightly job's every step passes locally from the new
paths (offline tests with and without the wall repo, the exporters,
`normalize.py all` → NOISE); the first Nightly run from the new layout
happens after the push — record its URL in `examples/wall/README.md`.

## Item 1 — undo/redo (days)

Design: DECISIONS.md rows 37 (revised 2026-08-19) and doc 13 §Undo.
- **WP-S (server)**: `OpLog` on the session (`VecDeque` of
  `{id, label, actor, before: (document, sidecar), timestamp}`, capped);
  every successful write pushes the pre-state `Session::handle` already
  clones for rollback; `undo`/`redo` intents restore and go through the
  normal persist + delta; `reload_from_disk` clears the log (barrier);
  effectful runs are not ops; lease-gated like every write.
- **WP-B (batch)**: the atomic multi-file edit for agents — shipped as
  the `apply_text` intent and `POST /api/edit/apply_text` (the name
  `batch` went to the canvas's gesture list, below):
  `{base_text_hash, files: [{path, text}], label, actor}` → refuse on
  stale base (`stale_base`, returns the current hash) or parse failure
  (diagnostics); else apply under the lock: write every file temp +
  rename, one op, one delta. `GET /api/edit/text` is the base to read.
  Multi-node canvas gestures (multi-move, multi-delete, reconnect) use
  the `batch {ops, label}` intent — a list of write gestures applied in
  order under the lock, all or nothing, one op, one delta.
- **WP-P (protocol + web)**: additive `history {can_undo, can_redo,
  undo_label, redo_label}` on Delta/Snapshot/`/debug/state`;
  `Ctrl+Z`/`Ctrl+Shift+Z`; toolbar buttons; Backspace removed from the
  delete keys; `#off` toggle on `D` (`writer::toggle_disable`,
  ports-intact ghost).
- **Done when**: session tests drive place → wire → set-param → delete →
  undo ×4 → redo ×4 and the text is byte-identical at every step; a
  `batch` with a stale base is refused and leaves disk + memory
  untouched (test asserts the file hash); a `batch` that fails mid-way
  leaves no partial file (fault-injection test); undo after a reload
  barrier is refused with the documented reason; Playwright: delete a
  node, `Ctrl+Z`, the node and its wires are back and the solved state
  is a cache hit (`/debug/state` shows `cached` for its outputs).
- **Shipped** (2026-08-20, `wt/undo`): all of the above plus the review
  riders — `Ctrl+Z` reaches the map from a focused slider (a range input
  is a control, not text entry), the one-op canvas gestures
  (multi-drag, target-anchor rewire) have Playwright coverage, a held
  `Ctrl+Z` past an empty side does not flood the notices — and `#off`:
  `writer::toggle_disable`, the `toggle_disable` intent, the ports-intact
  ghost (a `Line::Disabled` carries its parse), `D` / the node menu / the
  inspector action, `web/e2e/disable.spec.ts`.

## Item 2 — git panel slice 1 (~1 week)

Design: DECISIONS.md git row; doc 10 §Git integration; doc 13 routes.
- `git.rs` shells out to the git binary (`rev-parse`, `status
  --porcelain=v2 --no-optional-locks`, `diff -U0 HEAD`, `show
  HEAD:path`); typed states: not-a-repo, git-not-found, unborn,
  detached, `index.lock`. Project dir ≠ repo root is normal
  (`examples/wall`).
- Markers: hunks → binding lines → node names (the parser's spans);
  sidecar changes excluded from markers. `GET /api/git/status`;
  writer-gated `POST /api/git/commit` (message on stdin; scope = this
  pipeline's `.cic` + sidecar + `scripts/`) and `POST /api/git/revert`
  (to HEAD, through the existing barrier path).
- UI: TopBar branch/dirty chip; Git inspector tab with per-node markers;
  `Ctrl+S` commit dialog. HTTP-only — no WS message.
- **Done when**: markers equal `git diff` by construction (test: for a
  fixture diff, the set of marked nodes == bindings on changed lines); a
  status refresh never re-triggers itself on Linux (the 82df8a3 loop
  shape, CI test); commit from the app produces a commit `git log` shows
  with exactly the scoped paths; revert reaches the canvas within the
  measured barrier budget.
- Deferred inside the item: the canvas graph-diff overlay and `git log
  -L` per-node history — after the markers have weeks of use.

## Item P — the OCCT probe (hours, capped at one day)

A throwaway worktree with a private target dir; the deliverable is
`docs/probes/occt-2026-08.md` (written incrementally) plus the ledger
rows it forces. Questions, in order:
1. **Prebuilt build/link**: `opencascade-sys` with `--no-default-features`
   and `DEP_OCCT_ROOT` pointing at conda-forge `occt 7.8.1` (win-64 first
   — Ben's machine — then linux CI; macOS via CI). Report the Windows
   verdict within the first two hours. If MSVC fails, read the open
   upstream PRs (#230, #216) as starting points; keep only reviewed,
   minimal patches in an own fork.
2. **Determinism**: build the same box / extruded rectangle / boolean
   twice in two processes, serialize (BinTools, no triangulation),
   compare bytes; repeat across OSes in CI. This decides whether `Solid`
   hashes its canonical bytes or a geometric summary.
3. **Timings**: box, extrude, boolean, tessellate at 1 / 100 / 1,000
   parts, release build; cold CI minutes per OS with prebuilt OCCT.
4. **Policy**: LGPL-2.1 + OCCT exception row; `deny.toml` license and
   git-source exceptions scoped to the OCCT crates; the CI shape
   (`occt` cargo feature in `cicada-geom`, dedicated job).
**Done when** the memo answers all four with numbers and the ledger rows
are drafted; a red verdict is a valid outcome and comes back to Ben with
the fallback options.

## Item 3 — OCCT-backed Solid (weeks, from week 3 on a green probe)

Design: DECISIONS.md rows 16 and 42 (revised 2026-08-19), doc 03, doc 08
§7–8. B-rep is the default working mode; the wall stays on the mesh tier.
- **WP-A seam + build + CI**: feature-gated `occt` in `cicada-geom`, the
  prebuilt-OCCT fetch script (`tools/fetch_occt.py`, cache dir outside
  the repo, exports `DEP_OCCT_ROOT`), deny rows, the dedicated CI job.
- **WP-B the `Solid` kind**: canonical serialized bytes + blake3 at
  construction, geom-side handle cache, append-only `StoredValue`
  variant, script-boundary refusal (Python gets a typed "not
  marshallable" until a Solid ABI exists), web hue; display through a
  hash-keyed tessellation cache.
- **WP-C nodes**: `box`, `sphere`, `cylinder`, `cone`, `extrude`,
  `extrude_to_point`, `loft`, `revolve`, `sweep`, `pipe`,
  `solid_union/difference/intersection`, `volume`, `bounding_box`,
  `deconstruct_solid`, `section`, `tessellate → Watertight<Mesh>`
  (weld + watertight check + Manifold acceptance), `import_step`,
  `export_step` (effectful; header timestamps normalized). The shipped
  mesh-backed `box/sphere/extrude/loft` become `mesh_box/mesh_sphere/
  mesh_extrude/mesh_loft` in the SAME commit; `examples/`, the wall, the
  Playwright smoke and the measurement harness migrate with it.
- **WP-D consumer**: `examples/07-simple-cad.cic` (doc 01 use case 2)
  written before the nodes, solving at the end.
- **Done when**: the wall's cold carve and the 02-solids slider numbers
  are re-measured unchanged; golden hashes for transcendental-free
  solids pass on all three OSes (or the determinism policy the probe
  forced is recorded and applied); Esc during a long boolean is measured
  and written down, with the doc-12 kernel-worker named as the follow-up
  if it exceeds the 250 ms budget; `cargo deny` green; CATALOG.md
  regenerated.

## Item 3b — scheduler foundations + compute-on-release (~1 week, parallel)

- A per-solve cancel handle through `NodeFn` (not one session-global
  switch); a `volatile` `NodeDecl` flag beside `effectful` with executor
  gates at node and element granularity (Clock's "uncached by design");
  an idle-class hypothetical solve entry that paints nothing and is
  excluded from `wait_idle`.
- **Compute-on-release** (row 39): cones predicted ≥ ~1 s from the
  persisted cost samples show the pending value and an estimate during
  the drag and solve once on release. The wall's `deboss` is the test.
- **Done when**: virtual-time scheduler tests cover both gates; the
  deboss drag produces exactly one generation per release in
  `slider_loop.mjs`; the cheap-cone numbers (p50 0.5 ms) are unchanged.

## Item 4 — time transport, Cycle thin slice (~1 week)

Design: DECISIONS.md time row; docs 08 §1, 12, 13 §Animation transport.
`cycle(period, frames, frame: Integer = 0)` with the transport-driven
port hidden on the canvas; per-session transport state with an
injectable clock; play/pause/seek/speed intents + `TransportView` in
Snapshot (additive — PROTOCOL_VERSION stays); the frame injected through
the preview path (never the file); Esc pauses; Space toggles;
`examples/06-orbit.cic` in the same slice. Clock follows via `volatile`.
**Done when**: the second pass of a loop is 100 % `cached` with an
identical NodeKey set; "previews never write the file" holds under
playback (test); a headless run yields frame 0.

## Item 5 — scrub caching (1–2 weeks)

Design: DECISIONS.md row 39 (revised 2026-08-19), doc 12 §Speculative
warming. Opt-in per slider (`scrub=True` in the text — never the
sidecar), offered only when the step-quantized range has a bounded
position count (threshold set here and recorded in the ledger), off by
default, toggleable; a warming worker generic over param × ordered
value list (Cycle's playhead-ahead warming reuses it); nearest-first;
preempted by any real intent; a per-slider byte cap; one buffer-bar
component for both slider widgets. **Done when**: after idle every
position of the test slider is a memo hit; a step-snapped
`slider_loop.mjs` sweep reports every generation `cached` and the
client round-trip is compared against a MEASURED warm-restream floor;
the first drag tick mid-warm stays under the Esc p95.

## Item 6 — WASM script host (weeks, last)

Design: DECISIONS.md user-code row; doc 08 script nodes; doc 12
cancellation. Load precompiled `scripts/*.wasm` with a ports-manifest
custom section; postcard buffers in linear memory; wasmtime epoch
interruption wired to the per-solve cancel handle (3b); memory cap; no
WASI in the first slice; `cicada-guest` SDK + manifest macro (a tenth
crate — ledger row revised then); feature-gated `wasm`; one committed
fixture guest + `examples/08`; marshalling measured against the Python
bar. Compile-on-save is deferred explicitly. **Done when**: a guest node
solves in the app, an infinite-loop guest is killed by Esc within the
budget, and a crashing guest reds one node and nothing else.

## Track C — the catalog (continuous, parallel)

Design: DECISIONS.md stdlib row (revised 2026-08-19), doc 14 §node file
format, doc 08.
- **C0 restructure**: one node per file under
  `crates/cicada-stdlib/src/<category>/`, categories = ribbon tabs; the
  conformance test (title line, description, every port documented,
  `gh =` present, `# Examples` present and solving); `gh` attribute in
  `#[node]`; `# Examples` execution in CI. **Done when** the regenerated
  catalog is byte-identical to before the move, golden hashes unchanged.
  **Done 2026-08-20** in three commits: the catalog's within-category
  tie-break became the dialect name (so the move could be proven
  byte-identical and future moves never reorder the catalog), then the
  pure move (117 top-level items byte-identical, 58 golden constants
  unchanged, catalog byte-identical), then the format: `gh = "…" | none`
  required by the macro (trybuild cases), `# Examples` ```` ```cic ````
  fences extracted into `NodeSpec::examples` (bare fences refused —
  rustdoc would doctest them), all 57 nodes named or `none`d and given a
  runnable snippet, the conformance test
  (`crates/cicada-stdlib/tests/conformance.rs`), the runner
  (`crates/cicada-cli/tests/node_examples.rs` — parse, check with zero
  diagnostics, lower, solve with a fresh cache; exporters' inputs solve,
  the exporter itself is asserted skipped), CATALOG.md's `· GH:` tag and
  catalog.json's `gh`/`examples` fields, the `add-stdlib-node` skill
  rewritten for the layout. Honest reading of the DoD: "byte-identical"
  holds against the tie-break commit, not against the pre-C0 catalog —
  that one was ordered by source position (kinship order: add, subtract,
  multiply …), so the name tie-break permuted every category's lines
  once (a pure permutation, proven by sorted-line diff; the ribbon was
  never affected — the web client sorts each tab by title itself).
  **Review fixes (2026-08-20)**: `# Returns` (one line) is the doc of a
  bare single `out` port, required by the macro exactly when a node
  returns one bare value (three trybuild cases) — the first conformance
  test had exempted `out`, leaving 47 output ports undocumented in
  catalog.json; the "example calls the node" rule matches at an
  identifier boundary (`polyline(` no longer satisfies `line`); every
  node file now holds its own three tests (deconstruct_domain,
  deconstruct_point, flatten, mesh_difference, mesh_intersection gained
  theirs; sphere gained a transcendental-free topology golden) and the
  conformance test reads the source to enforce the layout
  (`src/<category>/<node>.rs`, one `#[node]` per file) and the three
  tests. **Pending (web lane)**: `gh` and `examples` are served by
  `/api/catalog` but the client's `CatalogNode` mirror and
  search-to-place do not read them yet; output-port docs likewise stop
  at the catalog (the view-model's `OutputView` carries no `doc`).
- **C1**: the nodes our diagnostics already cite — `compact`,
  `pad_last`/`repeat`/`truncate` policies — plus duplicate / reverse /
  sort / dispatch / group_by and the maths tail (min/max/abs/round/
  floor/ceil/trig). A test asserts no diagnostic names an unregistered
  node. **Done 2026-08-20** (`wt/catalog`, five node commits plus the
  review-fix commits, 48 nodes, all in the C0 format with the three
  tests, goldens blessed through the run-once path, catalog regenerated
  each commit): lists — `compact`, `duplicate`, `reverse`, `sort`,
  `dispatch`, `group_by`, `shift_list`, `split_list`, `nest`,
  `transpose`, `pad_last`, `truncate`, `weave`, `insert_items`; maths —
  `negative`, `absolute`, `round`, `floor`, `ceiling`, `min`, `max`,
  `sqrt`, `ln`, `log`, `exp`, `sin`, `cos`, `tan`, `asin`, `acos`,
  `atan`, `atan2`, `radians`, `degrees`, `smaller`, `larger`, `equals`,
  `and`, `or`, `not`, `xor`, `pick`, `mass_addition`, `average`,
  `bounds`; sequences — `range`, `repeat`, `jitter` (one shared PRNG).
  Decisions recorded: the cyclic zip policy is the node `repeat` (the §1
  time param owns `cycle`; DECISIONS.md GH-tree row revised 2026-08-20,
  docs/02/09/10 updated); `compact(list: [E?]) → (values: [E], map)` is
  honored at check time — an `E?` port keeps the wired `?` on the port
  (`bind_var`), so the "`compact` removes the holes" advice is
  satisfiable (checker fixture + a `[Number?]` script-node CLI test);
  `cross` (two type variables) and `squeeze`/`flatten_all`
  (data-dependent depth) wait on checker work.
  `crates/cicada-cli/tests/diagnostic_vocabulary.rs` scans the checker,
  scheduler, server (lowering, compile, session) and every stdlib node's
  string literals against the registry, with a PHANTOMS rule for the
  words that are not nodes (`cycle`, `cross`, `squeeze`, `flatten_all`,
  …) in any spelling. Port docs are whole paragraphs (the `Ports` macro
  joins a field doc's first paragraph; the conformance test reads a doc
  ending on a bare word as truncated).
  `examples/02-solids`/`03-voronoi` use `duplicate(count=1)` as the
  singleton (02 drops from 11 to 10 nodes; the Playwright smoke's count
  assertion moved with it); `examples/06-lists.cic` is the
  consumer (the orbit example planned as 06 moves to the next free
  number).
- **C2**: mesh-tier cylinder/cone/extrude_to_point/volume/bounding_box
  and the vector/plane nodes (ports pin_cutters / tip_caps math out of
  Python in the wall).
- **C3**: the core Curve ABI landed once (Arc/Ellipse/Compound,
  `Planar<Curve>`, Color authoring) then the curve nodes.
- **C4+**: the rest of docs/08 S+1, category by category, Solid rows
  riding with item 3.
- **`cicada mcp`**: catalog search, node docs, the checker — from the
  same data as `/api/catalog`; lands with C1.
Every node: the three tests, the doc format, catalog regenerated in the
same commit (skill `add-stdlib-node`).

## Gates that must not regress (re-measured at each geometry or scheduler landing)

From doc 15 §Stage-6 results: cold wall carve < 10 s (6.5 s), warm
< 100 ms (0.13 ms); cheap-cone slider p50 ≤ 16 ms / p95 ≤ 33 ms (0.5 /
1.4 ms); Esc time-to-idle p95 < 250 ms (214 ms); file edit → canvas
< 500 ms (~100 ms); wall output equivalence `overall NOISE` on Windows
and Linux.
