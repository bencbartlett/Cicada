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
| 3 | OCCT-backed Solid — the `Solid` kind, primitives/extrude/loft/revolve/sweep, booleans, `tessellate`, STEP; `mesh_*` renames in the same commit | main geometry track from week 3 | weeks | **WP-A done** 2026-08-20, review fixes applied the same day (fork `bencbartlett/opencascade-rs@960a8bc`, `occt` feature + seam in `cicada-geom`, `tools/fetch_occt.py`, CI jobs `occt (ubuntu)` per PR and `occt (<os>)` nightly — the non-Windows jobs await their first run); WP-B next |
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
- **WP-A seam + build + CI** — **done 2026-08-20**, review fixes the same
  day (docs/03 §The OCCT seam as built): feature-gated `occt` in
  `cicada-geom` over Ben's fork `github.com/bencbartlett/opencascade-rs`
  branch `cicada` @ `960a8bcb9e3dbf1916778dabb8288c1bda4c6d91` (upstream
  `d114250` + the MSVC handle aliases, the honest `BinTools`/`BRepTools`
  writers, in-memory serialization, the total exception boundary
  (`Standard_Failure` / `std::exception` / `catch (...)` → `Err`, with a
  per-clause self-test), the `cicada` glue, no OCCT source submodule — the
  git dependency costs 6 MB in `~/.cargo/git`, not 161 MB + 421 MB —
  and `LGPL-2.1-only` manifests); `tools/fetch_occt.py` +
  `tools/fetch_occt_manifest.json` (conda-forge 7.8.1 build 103 for
  win-64/linux-64/osx-64/osx-arm64 + the run-time closure, sha256-pinned,
  user cache dir, a warm path that re-verifies every shared library's
  presence and size, static closure check, typed network failures with a
  timeout, `--print-env`); deny rows for `opencascade-sys` + the fork
  source; CI jobs `occt (ubuntu)` (ci.yml, per PR) and `occt (<os>)`
  (nightly.yml, three OSes). Seam surface: `occt::Solid` with `box_at`,
  `extrude_polygon`, `difference`, `tessellate → Watertight<Mesh>`,
  `canonical_bytes`/`from_canonical_bytes`, every kernel call under one
  process-wide kernel lock; 21 tests incl. golden blake3 hashes of the
  canonical bytes, the weld refusals, the two-thread related-solids
  test and the per-clause boundary test. Measured on Windows; the
  Linux/macOS jobs are unverified until they run (the Linux job exports
  `LD_LIBRARY_PATH=<prefix>/lib` for the whole cargo step, which also
  shadows conda's libstdc++/libz/libexpat for the toolchain — probably
  fine, newer libstdc++ is backward compatible; switch to the macOS
  job's rpath approach if the first run says otherwise). Left for WP-B:
  the `Solid` value kind over `occt::Solid`, and the **sharing model**
  the kernel lock stands in for — OCCT results share `TShape`s with
  their inputs, so DISTINCT `Solid`s race in C++ when one is tessellated
  while another is serialized; choose deep copies at the seam
  (`BRepBuilderAPI_Copy`) or doc 12's kernel worker, then retire the
  lock; per-OS goldens until the nightly shows agreement (the policy is
  in DECISIONS.md rows 16 and 42, revised 2026-08-20 at the merge of
  WP-A from the probe memo's §4d drafts);
  for WP-C: the own-built OCCT with FreeType/FreeImage off (the fetch
  table takes a second source), `Message_Printer` redirection for the
  STEP writer, the patches for the static path (PR #216's system libs,
  cmake-rs `/O2`) if it is ever taken.
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

- **Control-plane priority over the display restream** — **done on the
  server and the socket, 2026-08-20** (`wt/hardening`; docs/13 §Two
  lanes, one socket); **the status cadence at wall scale is NOT reached
  on the page** and is recorded as the display plane's follow-up, not
  claimed. A client that joined received the whole display set on the
  one socket (the wall: 26 frames, 368 MB, the largest 94 MB) and every
  text — `preview_policy`, deltas, statuses — queued behind it:
  `preview_policy` reached a fresh observer ~26 s after the drag on a
  loaded dev machine. Shipped as the smaller of the two options doc 13
  named: two channels per client (`ClientLanes`: control and display)
  and a write task that drains control first, the display lane FIFO —
  `display_reset` and `screenshot_request` ride the display lane because
  their meaning is their place among the frames; nothing on the wire
  changed (`PROTOCOL_VERSION` unchanged). Plus the join-time half the
  review found: the write task starts before the restream is built
  (`attach_client`), `Session::join` hydrates under the registration's
  lock hold, and `restream_display` encodes outside the session lock
  with a per-output compare-and-send (the pick table behind its own
  mutex) — a joiner used to see nothing for ~3 s while every other
  client's intents waited on the lock. The "frame bus drops
  stale-generation frames" premise was half right and is recorded
  precisely in docs/13: staleness is per output, and a rule keyed to the
  reset's generation would drop a restream's unchanged outputs; the web
  test pins the interleavings that ARE safe. Tests: the pump (priority,
  FIFO, the `biased` select pinned by 64 messages per lane); a wall-sized
  synthetic restream (the 94 MB frame in flight, 319 MiB behind it) + a
  slider tick whose status goes out behind exactly the frame in flight;
  the parked-restream join (hydrated, intents answered, the superseded
  output not resent); the lane assignment of the two display-plane
  texts; the HTTP e2e's join order — all on a permit-paced recording
  sink, no sleeps. Measured with `tools/measure/lanes.mjs` (the wire, no
  browser; the "before" engine built from `24d558b`'s server sources):
  a tick at the observer's snapshot reaches it after 368 MB / 278 ms
  with one queue, behind 48 KB / 1.3 ms with the lanes; socket open →
  `hello` 3,031 ms → 7 ms; a tick 50 ms into a join is answered in
  3,202 ms → 1.3 ms. End to end in `compute_on_release.spec.ts`
  (headless Chromium, software GL; the observer grabs mid-restream; the
  spec now MEASURES — logs, attaches, annotates — under a 60 s sanity
  bound): observer `preview_policy` after the grab — paired runs in one
  session, 2026-08-20, debug engines: **21.0 s** with one queue (the
  `24d558b` engine; 14 of 26 frames in at the grab, all 26 at the hint)
  → **5.9 s and 11.7 s** with the lanes and the join fix (24 and 21
  frames in at the grab, all 26 at the hint); earlier runs 15.6 s →
  3.5–8.9 s. The writer's own hint 160–176 ms in both (one 4.5 s
  outlier: the two pages share one headless browser). The residual is
  the page's own message queue: the
  browser takes the whole restream in faster than it handles frames,
  so a text sent once the server has written them is legitimately last
  on the wire — no socket order can fix that, and the page cannot be
  the socket's oracle (the order guard the spec used to carry was
  valid only while the tick beat the server's ~3 s build). Whether the
  page's seconds per 27–94 MB frame are software-GL renders or
  decode/upload is unmeasured; "a GPU browser pays milliseconds" was a
  hypothesis, not evidence. Next, named, not scheduled: frame handling
  off the main thread (decode in a worker → typed arrays to the scene),
  the one change that lets the queue drain at memcpy speed;
  chunked/element-range frames; a per-output latest-wins display queue
  for display-vs-display blocking; the live `emit_frames` still encodes
  under the session lock (changed outputs only).
- **A count/allocation guard for every count-taking node** — **done
  2026-08-20** (`wt/hardening`, three commits; docs/08 rule 7 is the
  contract). A slider wired into `count` could ask for a capacity the
  allocator refuses, which aborts the process — `catch_unwind` cannot
  catch it. The 2^24 slot ceiling (15112fb) already stood on eight count
  ports; this package gave it its byte half and its product form,
  audited every node that allocates from a count, and then — after the
  adversarial review measured what the ceilings really admitted —
  lowered the slot half, charged per-copy payloads, counted text spans
  instead of flattening, and bumped the versions. `slot_count` became
  `checked_count(node, port, value, least, bytes_per_slot)`: the same
  floor with the same messages (the run_e2e regression still matches),
  `MAX_SLOTS` = 2^22 (4,194,304), and `MAX_BYTES` = 1 GiB on `count ×
  bytes_per_slot`. Why 2^22 and not 2^24: the review measured the
  end-to-end cost of a slot (value-model hashing + memo log + zstd) —
  `series` at 2^24 peaked at 9,763 MiB of working set and wrote 1.4 GB
  to the cache, ~580 bytes a slot for an 8-byte element — so the 2^24
  band let a slider reach the allocator-failure abort on an 8–16 GB
  machine a few million slots UNDER the ceiling; at 2^22 the process
  peaks at 2,478 MiB in 4.1 s (measured the same way; the numbers are
  on the constant). `bytes_per_slot` is what a slot makes the node
  allocate, not the element's `size_of` alone: `linear_array` charges
  each copy its `Transformable` plus the mesh or polyline it transforms
  (`transform::support::payload_bytes`; every copy is a fresh geometry —
  the review measured 3.5 GB committed for 100 copies of a 24 MB sphere
  against a guard counting 11,200 bytes), so a million-vertex mesh is
  refused at 30 copies; `duplicate`'s `Arc`-shared slots stay the slot
  alone. `checked_size(node, what, slots: u128, bytes_per_slot)` is the
  same check on a derived count, for the nodes whose allocation is a
  PRODUCT of inputs: the sphere's `segments × rings` vertices (2,897
  segments is the last allowed, 2,898 the first refused — 4,196,306
  vertices; `segments = 10^14` is refused with its 5 × 10^27 in the
  message, u128 so no overflow), and the text nodes' span bound — now
  from `Font::outline_spans` (cicada-geom), which counts the outline
  callbacks without flattening: a contour start or a line span is at
  most one vertex at any density, a bézier span at most `segments`, so
  the bound holds by construction of the flattener (it only ever drops
  vertices), a line-only glyph (`A`) is never refused for its density,
  and the single-closed-loop contour a two-chord counting pass dropped
  is counted like any other span; asserted over every glyph of every
  bundled face at 1, 2, 3, 8 and 64 chords, and against a synthetic
  loop / square / degenerate span in the geom tests. `extrude` / `loft`
  / `voronoi` police `segments` only where it sizes an allocation (a
  circle profile, an analytic section, a circle boundary); a chain
  profile never tessellates and its unused port is not policed. Audited
  and left alone, with the reason: `chunk`, `partition`, `truncate`,
  `split_list`, `shift_list`, `item`, `weave`, `insert_items` allocate
  no more than their input; `jitter`'s integer is a seed. Floors stay
  where they were (the node's or the kernel's message). **Versions
  bumped to 2** on the fourteen nodes whose previously-valid band is now
  red (`series`, `range`, `random`, `repeat`, `duplicate`, `pad_last`,
  `divide_curve`, `linear_array`, `sphere`, `extrude`, `loft`,
  `voronoi`, `text_outlines`, `text_solids`): docs/12 says any behaviour
  change, and the review reproduced the alternative — one binary serving
  a memo hit for `text_outlines(segments=2000000)` that a cold solve
  refuses; the wall recomputes once (cold carve 3.8 s). Golden hashes
  unchanged (value hashes, not keys). Tests: message-exact unit tests
  for both helpers at both ceilings (inclusive bounds, the overflow-proof
  product); per node a case one past the ceiling that bites first (red
  with the exact message) plus, where the port is unused for a chain
  input, the same count building the same mesh it always did; the
  sphere's vertex formula pinned to the kernel's count; `linear_array`'s
  fat-mesh case (36 MB × 30 refused with the per-copy bytes in the
  message, 2 copies built), its polyline-vs-circle case and its
  slot-ceiling case; the text bound pinned within 2× of what the layout
  produces. "No allocation" is proven by the absurd-count cases — with
  the guard moved after the allocation, `series(10^11)`, `sphere(10^14)`,
  `linear_array(10^11)` and now `text_outlines("O", 10^11)` /
  `text_solids("O", 10^11)` (~10^12 outline vertices) cannot complete:
  the review's mutation showed the earlier line-only `A` cases proved
  nothing (an `A` is eleven line vertices at any density), and the `O`
  mutation was re-run here — the test binary was still growing at
  8.5 GB after 15 s and was killed. The cap+1 cases pin the boundary and
  the message; only the absurd cases detect a guard-after mutation. A
  test-only counting `#[global_allocator]` would have proven it at
  exactly cap+1 but needs `unsafe` outside an FFI seam, which the rules
  forbid — recorded as the one assertion not made.
- **Tessellation `segments` bound memory, not time** (found by the
  guard review, verified 2026-08-20): `extrude` of a circle at 50k
  segments takes 2.0 s, 100k 8.7 s, 200k 37 s (release; the cap ear clip
  is O(n²)) — everything under the 2^22 ceiling is admitted and a slider
  past ~10^5 is an hours-long, uncancellable solve (Esc cannot interrupt
  an in-kernel call today). A `segments`-only ceiling would be partial
  (a 200k-vertex polyline profile from `divide_curve` reaches the same
  clipper), so this is the cost model's and the cancellable kernel
  worker's (docs/12): a per-node cost estimate before the call, and a
  convex fast path in the clipper. Until then, the numbers are in docs/08
  rule 7.
- **CI solves every `examples/*.cic`** — **done 2026-08-20**
  (`wt/hardening`; `crates/cicada-cli/tests/examples_solve.rs`, the rule
  stated in `examples/README.md`). Only `02-solids` was exercised (by the
  Playwright smoke). Now every `examples/**/*.cic` — discovered by
  extension, never listed, so a new example is covered the moment it is
  committed — solves in-process through the same `compile::load` →
  `resolve_targets` → `lower` → `Scheduler::solve` path `cicada run`
  prints over, with a fresh `DiskStore` per example (nothing can be a memo
  hit, so every node really computes) and no `--node` (every non-effectful
  leaf; exporters skipped, their inputs solved, and the test asserts they
  stayed `Skipped`). Green means: zero checker diagnostics anywhere
  (stricter than `run`'s cone gate on purpose — a warning outside the cone
  is still a wrong example), zero red, zero blocked; the failure names the
  example and the binding in `run`'s own words (`red \`xs\` — range: steps
  must be >= 1, got 0`, `blocked \`n\` — fed by red \`xs\``; verified by
  mutating `06-lists`). The wall IS included: measured cold in debug on
  the 24-core dev machine 6.9 s at cores − 2, 18 s at 4 threads, 34 s at 2
  — the whole test runs in 7.8 s here; CI's 4-vCPU runners will sit near a
  minute, and the test's header says where the exclusion list goes if
  that ever dominates. The runner's own contract is pinned (a red binding,
  its blocked dependant and a diagnostic are each reported, never
  skipped), as is discovery (the nested `wall/wall.cic` and a floor of
  seven files).
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
