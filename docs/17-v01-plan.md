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
| 2 | Git panel slice 1 — status strip, per-node change markers, commit, revert-to-HEAD | foreground (server/web) | ~1 week | **done** 2026-08-20 (wt/git-panel): `GET /api/git/status` + writer-gated `POST /api/git/commit` / `POST /api/git/revert` (docs/13), the web chip / Git tab / canvas badges / `Ctrl+S` commit dialog (docs/16); measured, debug builds: revert POST → barrier snapshot ≤ 35 ms (route test), Revert click → reloaded text in the store 69–81 ms across runs (Playwright) |
| P | OCCT probe — prebuilt 7.8.x build/link on win/mac/linux, determinism, timings, license, CI shape | parallel worktree | hours, cap 1 day | **done** 2026-08-20 — GREEN on win-64 with one rename patch, byte-deterministic, ~3 ms/boolean; Linux/macOS measured by item 3 WP-A's CI job; memo `docs/probes/occt-2026-08.md` |
| 3 | OCCT-backed Solid — the `Solid` kind, primitives/extrude/loft/revolve/sweep, booleans, `tessellate`, STEP; `mesh_*` renames in the same commit | main geometry track from week 3 | weeks | **unblocked** (probe GREEN) — WP-A next: own fork with the recorded patches, `occt` feature, `tools/fetch_occt.py`, the per-OS CI job |
| 3b | Scheduler foundations — per-solve cancel handle, `volatile`, idle-class hypothetical solve — plus compute-on-release | parallel (sched/server) | ~1 week | **done** 2026-08-20 (`wt/sched`, eighteen commits after three review rounds: the engine half, then the web half — both sliders show the pending value + estimate from `preview_policy`, the release that writes nothing is `end_drag` and every announced drag's end is `drag_ended` — with a Playwright drag of the wall's `deboss`, an observer page watching, as its evidence) |
| 4 | Time transport — Cycle thin slice + orbit example; Clock via `volatile` | foreground | ~1 week | pending |
| 5 | Scrub caching — bounded-position sliders only, toggleable, buffer bar | foreground | 1–2 weeks | pending |
| 6 | WASM script host — load precompiled guests, epoch cancellation, `cicada-guest` SDK | last | weeks | pending |
| C | Catalog — one-node-per-file restructure, node-format conformance test, then the docs/08 S+1 list in tranches; `cicada mcp` | parallel worktrees, continuous | continuous | **C0 done** (2026-08-20); **C1 done** (2026-08-20: 48 nodes — lists, maths tail, sequences; the diagnostics name real nodes and a test keeps it so; `compact` satisfiable at check time; `examples/06-lists.cic`); **`cicada mcp` done** (2026-08-20: the four doc-11 read tools over stdio on `rmcp`); C2+ pending |

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
- **Shipped 2026-08-20** (wt/git-panel; docs/13 §HTTP surface rows,
  docs/16 Git panel bullet, doc 10 §Git integration): every "done when"
  holds — markers vs `git diff` (`git_routes.rs`
  `markers_are_exactly_the_bindings_on_the_diffs_changed_lines`), the
  no-self-retrigger status (`--no-optional-locks`, `.git/index`
  byte-identical across refreshes), commit = exactly the scope (route
  test + Playwright `git log`), revert → barrier ≤ 35 ms server-side and
  click → canvas 69–81 ms in the browser (debug builds). The scope carries
  `in_head` per file — the revert rule is the server's, published, not
  re-derived by the client (review finding: porcelain `AD` is `deleted`
  with no HEAD version).

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
- **Shipped (engine half, 2026-08-20, `wt/sched`)**: (1) the cancel
  handle — `NodeFn` takes the generation's `NodeCtx` (its
  `CancelToken`, now with `on_cancel` hooks); the script bridge mints
  one kill switch per call hooked to the calling token, so explicit
  runs, the interactive loop and idle solves are isolated by
  construction (the old session-global switch let an Esc kill an
  export's Python call); (2) `#[node(volatile)]` → `NodeSpec`/`NodeDecl`
  with node- and element-level memo gates, downstream keyed as usual on
  the fresh hash (doc 12 §Volatile nodes), `volatile`+`effectful`
  refused by the macro (trybuild), a cfg(test) fixture node,
  `"volatile"` in catalog.json; (3) `SolveLoop::run_idle` +
  `Session::solve_hypothetical` — idle class, pre-empted by any real
  submission or Esc, invisible to `wait_idle`, paints nothing, one
  `hypothetical` timing row; (4) compute-on-release — decided PER TICK
  from a hash-only dry run of the tick's keys against the memo (warm
  values stay live; a cold tick is withheld whatever the drag's first
  tick was), monotone within a drag, announced once per drag by the
  additive `preview_policy` message (doc 13 §Slider drags has the
  frozen shape and the drag-gap rule: a drag ends on a write attempt,
  an Esc, or a 300 ms pause, and the next one is announced again),
  `COMPUTE_ON_RELEASE_MS` = 1 s; memo entries record their
  computation's cost so the model is complete after a warm reopen.
  Review fixes 2026-08-20: the warm-first-tick and no-write-drag holes
  closed (both reproduced on the wall), the Python bridge wiring pinned
  by an end-to-end Esc test. Measured: 02-solids `size` warm p50/p95
  0.22/0.82 → 0.23/0.84 ms server, 0.42/1.4 → 0.43/1.42 ms client
  (within noise); wall `deboss` 301 ticks → 0 preview generations, 1
  policy message (estimate 3.9 s), 1 generation on release (3.7 s);
  after a warm reopen the estimate is 4.1 s from memo costs alone and
  the released value previews live at 0.2 ms. `slider_loop.mjs` gained
  the compute-on-release mode and `--expect`. Re-measured after the
  review fixes (per-tick prediction on every tick): 02-solids `size`
  warm 0.24/0.84 ms server, 0.45/1.47 ms client; wall `deboss` 301
  ticks → 301 withheld, 0 preview generations, exactly one policy for
  the stream (estimate 3.9 s), one 3.5 s generation on the step-snapped
  release. Second review round (2026-08-20): the "failure under a
  cancelled token is cancelled" rule narrowed to errors the node MARKS
  as cancellation (`NodeError::cancelled`; the bridge's verdict for a
  killed worker) so a genuine red coinciding with Esc stays red; the
  drag is ended in one place (the dispatcher's door, for every write
  intent but the tick) with undo / redo / a refused batch pinned as
  drag-enders; the inclusive 1 s bar, the `÷ min(threads, elements)`
  divisor and the volatile memo-READ gate each got the test whose
  absence a mutation had exposed; a `cached` status carrying its last
  compute's `elements`/`nanos` is a decision now (doc 13 §Solve
  streaming), not an open question; the store's format marker is
  written temp + rename and an empty (torn) one heals.
- **Shipped (web half, 2026-08-20, `wt/sched`)**: both sliders (canvas
  and params panel) show the pending value — thumb and number in the
  warn color — and a `pending · N s` chip (`~` when `rough`, the ETA's
  spelling) from `preview_policy`; the store holds ONE pending param
  (the server holds one drag), every arrival replaces it, and it
  clears on the delta of the release's `set_param` (any write ends the
  drag), on a refused write's `error` (a `lease` refusal excepted — it
  is decided before the drag-ending door), on a snapshot and on a
  disconnect; a release on the committed value writes nothing, so the
  widget clears it itself AND sends `end_drag`, and the server's
  `drag_ended` takes it down everywhere else (see the review round
  below); frames and statuses never clear it (a memo-warm tick paints
  live mid-drag by design); observers render the same from the
  broadcast; `cached` statuses read `cached · last 43.9 s`. Evidence:
  store unit tests for every transition and
  `web/e2e/compute_on_release.spec.ts` — a real pointer drag of the
  wall's `deboss` on the debug engine (2 threads): cold open 31 s,
  estimate 23 s, 9 ticks withheld, 0 computing previews, the hint up
  while held, one 29.6 s generation on release (1.2 min on the dev
  machine; it is the suite's slow spec by design and carries its own
  timeout). Re-measured at the final engine: 02-solids `size` warm
  0.24/0.89 and 0.24/0.72 ms server p50/p95 (0.44/1.52, 0.43/1.33 ms
  client); wall `deboss` 301 ticks → 301 withheld, 0 preview
  generations, 1 policy for the stream (estimate 3.6 s), one 3.6 s
  generation on release; warm reopen estimate 3.9 s, release 0.22 ms
  all cached.
- **Review round on the web half (2026-08-20)**: three holes, one
  contract revision (doc 13 §Slider drags, the "end of an announced
  drag is announced" paragraph; DECISIONS.md row 39). (1) The params
  panel's slider ended its drag only from the native `change` event,
  which Chrome does not fire when a drag returns to its start value —
  the badge stood for ever after such a drag (the canvas widget, which
  releases from pointer events, did not have the bug); both sliders
  now decide the no-write release on the pointer's / key's release.
  (2) Observers (and the non-dragging twin widget, and a writer whose
  release is refused by the lease) had no signal at all for a drag
  that ended without a write: the frozen contract's "the server never
  sends back-to-live" left them a stale badge and a value that was
  neither committed nor pending. Decided: the release that writes
  nothing is an intent — `end_drag` — and the server broadcasts
  `drag_ended` whenever an ANNOUNCED drag ends (after the delta /
  error / snapshot when there is one; alone for `end_drag`, Esc, the
  writer's departure, a lease handover), the gap rule's end excepted
  because a pause is not a release and the badge must not flicker off
  under a held pointer. (3) A re-grab inside the 300 ms gap after a
  no-write release continued the server's drag un-announced while the
  client had already cleared — `end_drag` ends the server's drag at
  the release, so the re-grab is a fresh drag, announced again. Tests:
  session tests for every end and both clients (`an_announced_drags_
  end_is_announced_to_every_client`, `the_writers_departure_and_a_
  lease_handover_end_the_drag_for_the_observers`), the protocol shapes,
  store tests for `endDrag` / `drag_ended`, and — new — jsdom +
  Testing Library component tests for BOTH sliders (the chip and the
  value while pending, the pending value following the thumb, the
  drag-return release, the write release, the keyboard path, the
  observer view; they fail on the review's mutations), and the e2e
  grew a second half: an observer page, a real pointer drag away and
  back to the committed value (28 more ticks withheld, both pages'
  badges down on release, the server's drag ended, nothing written,
  nothing solved), the re-grab announced again, the canvas twin's chip
  and value asserted from the DOM (1.7 min on the dev machine). The
  `cancel` on the canvas sender drops a queued tick on a no-write
  release (sent after the `end_drag` it would be a fresh drag on the
  committed value — cold while its own carve is still running). Still
  not relayed: the writer's later ticks to observers (they see the
  policy's first withheld value with the badge until the release —
  doc 13 contract item 4).

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
  tests. **Web lane (2026-08-20)**: the client's `CatalogNode` mirror
  reads format 2 (`gh`, `examples`, per-port `doc`; a vitest pins it to
  the committed `catalog.json`); search-to-place — the double-click box
  and the wire-to-empty-canvas box — matches name, title AND `gh`,
  ranked name exact > gh exact > title exact > prefix > substring
  (`Addition` → `add` above `mass_addition`), and shows `GH <name>` on a
  row whose GH name differs from its title; port hovers on the canvas
  and in the inspector read `name: type — doc`, output docs looked up
  in the catalog by func (the view-model's `OutputView` still carries
  no `doc` — deliberately: the catalog is the source either way).
  `web/e2e/search.spec.ts` drives it through the real app.
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
  same data as `/api/catalog`; its own package after C1 (C1 shipped
  without it). **Done 2026-08-20**
  (`wt/catalog`, one commit): `cicada mcp [--project <dir-or-pipeline>]`
  is a Model Context Protocol server over stdio on `rmcp` 3 (the
  official Rust SDK, Apache-2.0; `server` + `transport-io` features
  only — no macros, the tools are plain routed functions the workspace
  lints see; 17 new crates in the native build graph — 32 lockfile
  entries counting the platform-only ones (wasm-bindgen / js-sys,
  windows-core, core-foundation, android, haiku) — every one MIT
  and/or Apache-2.0, `cargo deny check` green, no new duplicate
  versions). Tools: `catalog_search {query, category?,
  limit?}` (per-word scoring over name / title / `gh` / ports /
  description, exact > prefix > contains, prose at word starts only;
  empty query lists), `node_doc {name}` (the `/api/catalog` node object
  from the server's own renderer — `catalog::node_value`, an additive
  accessor — plus `signature` and `effectful`; unknown names get the
  checker's did-you-mean by checking a one-binding probe), `list_categories`,
  `check {text | path}` (the doc-11 diagnostics from
  `compile::check_source`, the new shared entry `load` and the session's
  `reload_text` now call — one checker). `--project` discovers the
  project's scripts through `compile::catalog_specs_in` /
  `scripts::discover_in` (additive) and re-discovers when the bytes of
  `scripts/*.py` change; a broken `scripts/` refuses at startup.
  Refusals are structured tool errors, stdout is protocol-only, the
  server exits on EOF. Tests: `crates/cicada-cli/tests/mcp.rs` drives the
  built binary with JSON-RPC framing (handshake, `tools/list` shapes,
  every tool, the `slider` doc's ports + GH name, the did-you-mean
  diagnostic, `--project` script nodes + relative paths + live
  re-discovery, the startup refusal) plus unit tests in `mcp.rs`.
  `.mcp.json.example` registers it for Claude Code; docs 11 and 14 and
  the AGENTS.md palette carry the surface. Deliberately NOT in this
  slice: the live-graph tools (`what_feeds`, `wire_summary`, `profile`,
  `diagnostics`) — they need a running solve and belong to the app's
  server; a `volatile` flag in `node_doc` — `NodeSpec` has none until
  item 3b adds it (only `pure` / `effectful` are reported; when 3b adds
  the field to `catalog::node()`, the `node_doc_schema_matches_every_catalog_entry`
  test demands it be described in `NodeDoc` in `mcp.rs` — the value
  flows through the renderer on its own, the schema does not).
  **Review fixes 2026-08-20** (adversarial review of the package):
  `check` now runs the dry lowering (`lower_partial`, the session's
  form) after the checker and reports `excluded` bindings with the
  canvas's status + reason (`Exclusion::reason()`, one renderer shared
  with the view-model) — `ok` was true for an integer literal at 2^53
  that `cicada run` refused; `node_doc`'s `outputSchema` is the real
  node shape (`NodeDoc` / `PortDoc`, held to `catalog::node_value` by a
  test over every stdlib node) instead of an open object; the
  project's script nodes are KEPT in the catalog cache (the dry lowering
  needs their run functions to exist; nothing ever runs); tests now pin
  GH-only search matches (`Pick'n'Choose` → `pick`, `addition` → `add`
  above `mass_addition` — deleting the `gh` scoring branch once left
  every test green), the out-of-project and scripted-subdirectory
  `check {path}` branches (forcing the in-project branch once passed
  every test), an exporter's `pure` / `effectful`, and the 2^53 literal.
  `.mcp.json.example` registers the BUILT binary
  (`${CARGO_TARGET_DIR:-target}/debug/cicada`, Claude Code's variable
  expansion) — the `cargo run -p cicada-cli` form is a cold
  feature-unification context that rebuilds the Manifold kernel with
  cmake on PATH, which no MCP client's startup timeout survives; `.mcp.json`
  is gitignored (the docs say to copy the example there).
Every node: the three tests, the doc format, catalog regenerated in the
same commit (skill `add-stdlib-node`).

## Follow-ups (found by the v0.1 reviews and measurements; scheduled, not yet placed)

- **Control-plane priority over the display restream.** A client that joins
  a session receives the whole display set on the one socket (the wall:
  ~350 MB of binary frames, measured 2026-08-20) and every text frame —
  `preview_policy`, deltas, statuses — queues behind it; on a loaded dev
  machine `preview_policy` reached a fresh observer ~26 s after the drag.
  doc 13 named head-of-line blocking as the trigger for a transport change:
  the fix is a priority lane for text frames (two channels per client with
  text-first draining — the frame bus already drops stale-generation frames
  — or a second socket for binary), with a test that a text frame sent
  behind a multi-hundred-MB restream arrives within the status cadence.
  Protocol-change skill; one package. Until then
  `web/e2e/compute_on_release.spec.ts` waits for delivery, not latency.
- **A count/allocation guard for every count-taking node** (`series`,
  `range`, `duplicate`, `repeat`, …): a slider wired into `count` can ask
  for a capacity the allocator refuses, which aborts the process —
  `catch_unwind` cannot catch it. A shared `checked_count(count,
  bytes_per_slot)` that refuses loudly (red, with the number) before
  allocating; one package in stdlib + a table test per node.
- **CI solves every `examples/*.cic`.** Only `02-solids` is exercised (by
  the Playwright smoke); a `cicada-cli` test that runs each example headless
  with a fresh `--cache-dir` keeps `06-lists` and the rest solving.
- **Renumber item 4's orbit example**: `examples/06-lists.cic` took the
  number; the transport slice uses the next free one.
- **Stale catalog on the client after a scripts-change reload**: the app
  fetches `/api/catalog` once; search rows and port tooltips for script
  nodes go stale until a reload. Refetch on the catalog-reload barrier.

## Gates that must not regress (re-measured at each geometry or scheduler landing)

From doc 15 §Stage-6 results: cold wall carve < 10 s (6.5 s), warm
< 100 ms (0.13 ms); cheap-cone slider p50 ≤ 16 ms / p95 ≤ 33 ms (0.5 /
1.4 ms); Esc time-to-idle p95 < 250 ms (214 ms); file edit → canvas
< 500 ms (~100 ms); wall output equivalence `overall NOISE` on Windows
and Linux.
