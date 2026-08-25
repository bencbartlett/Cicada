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
| 0 | Fold `corpus/` into `examples/wall/` + `tools/`; this plan | foreground | hours | **done** 2026-08-20 (every nightly step passes locally from the new paths); the first Nightly runs from the new layout (2026-08-22..24, run 32562121114 first) carve the wall on the runner and compare it to production — `overall NOISE` all three nights — and the 3-OS matrix's tests pass; two jobs were red all three nights for reasons outside the engine — addressed 2026-08-24, the heavy job's green run still owed once the account's Actions budget is back (§Follow-ups, "The first Nightlies") |
| 1 | Undo/redo — snapshot op log + atomic `batch`/`apply_text` path; riders `#off`, Backspace-no-delete | foreground (server/web) | days | **done** 2026-08-20 (merged) |
| 2 | Git panel slice 1 — status strip, per-node change markers, commit, revert-to-HEAD | foreground (server/web) | ~1 week | **done** 2026-08-20 (wt/git-panel): `GET /api/git/status` + writer-gated `POST /api/git/commit` / `POST /api/git/revert` (docs/13), the web chip / Git tab / canvas badges / `Ctrl+S` commit dialog (docs/16); measured, debug builds: revert POST → barrier snapshot ≤ 35 ms (route test), Revert click → reloaded text in the store 69–81 ms across runs (Playwright) |
| P | OCCT probe — prebuilt 7.8.x build/link on win/mac/linux, determinism, timings, license, CI shape | parallel worktree | hours, cap 1 day | **done** 2026-08-20 — GREEN on win-64 with one rename patch, byte-deterministic, ~3 ms/boolean; Linux/macOS measured by item 3 WP-A's CI job; memo `docs/probes/occt-2026-08.md` |
| 3 | OCCT-backed Solid — the `Solid` kind, primitives/extrude/loft/revolve/sweep, booleans, `tessellate`, STEP; `mesh_*` renames in the same commit | main geometry track from week 3 | weeks | **WP-A done** 2026-08-20, review fixes applied the same day (fork `bencbartlett/opencascade-rs@960a8bc`, `occt` feature + seam in `cicada-geom`, `tools/fetch_occt.py`, the dedicated `occt` CI jobs — folded by WP-C into the standard per-PR `rust` / `test-cross` / `playwright-smoke` jobs and the nightly matrix, every building job fetching the prebuilt first; the non-Windows runs await the branch's first CI run); **WP-B done** 2026-08-20 (`wt/solid`: the `Solid` kind end to end, the sharing model — op-local linear handles, no kernel lock — the value-level `cicada_geom::solid`, display through the session's `SolidCache`, the typed Python refusal, the store variant with a committed pre-change pack; the handle cache measured and NOT built); **WP-B second review closed** 2026-08-21 (`wt/solid`: the moved-sphere stale-pcurve root cause fixed in `transform`, display draws unclosed meshes and says so, display tiered + off the session lock on the worker pool, blobs keyed by the display mesh's hash, typed `NotOneSolid`, cached refusals, the 02-solids display cone at 5.2 ms p50 — §Item 3 has the table); **WP-C + WP-D done** 2026-08-20 (`wt/solid`: `occt` ON by default + every CI job fetches the prebuilt; the node-set glue in cicada-geom; `box`/`sphere`/`cylinder`/`cone`/`extrude`/`extrude_to_point`/`loft`/`revolve`/`sweep`/`pipe`/`solid_union`/`solid_difference`/`solid_intersection`/`volume`/`bounding_box`/`deconstruct_solid`/`section`/`tessellate`/`export_step`/`import_step`; the mesh tier as `mesh_*`, the wall's carve hash unchanged; `examples/07-simple-cad.cic` + its Playwright spec; `mirror` added 2026-08-21; numbers below — the cheap-cone slider on the OCCT example: the display cone PASSES since the second review (5.2 / 9.1 ms) while the COMMITTED 02-solids misses the 16 ms bar because its export `tessellate(deflection=0.01)` node sits in the cone (34 ms per tick — a one-line decision for Ben), and Esc inside ONE kernel call misses 250 ms: both named follow-ups); **WP-C/WP-D review closed** 2026-08-21 (`wt/solid`: the tier flip's cache hygiene — `box`/`sphere`/`extrude`/`loft` at version 2 with a stale-memo regression test and a committed signature ledger that makes "a changed meaning bumps the version" a test; `tessellate` bounded by a budget before the mesher runs; `section` tells a tangent contact from a loop; the stdlib's kernel-free world is real and tested; the MCP registration carries the loader path; §Item 3 has the record) |
| 3b | Scheduler foundations — per-solve cancel handle, `volatile`, idle-class hypothetical solve — plus compute-on-release | parallel (sched/server) | ~1 week | **done** 2026-08-20 (`wt/sched`, eighteen commits after three review rounds: the engine half, then the web half — both sliders show the pending value + estimate from `preview_policy`, the release that writes nothing is `end_drag` and every announced drag's end is `drag_ended` — with a Playwright drag of the wall's `deboss`, an observer page watching, as its evidence) |
| 4 | Time transport — Cycle thin slice + orbit example; Clock via `volatile` | foreground | ~1 week | **DONE** 2026-08-20 (`wt/transport`): engine — `cycle` / `clock` with the `transport_driven` port attribute, the playhead injected at lowering, per-session transport state + the five `transport_*` intents + `TransportView` in every snapshot and the `transport` broadcast, playback over the preview loop, `examples/08-orbit.cic` (orbit second pass 120 generations, 0 computed / 1,800 cached, p50 0.43 ms); web — the play bar (play/pause, the frame scrubber, speed, reset), `Space`, the transport-driven ports hidden on the canvas and in the inspector (each driven port carrying its own loop; the server owns the wire-target rule — `probe_wire`/`connect` refuse), observers read-only, `web/e2e/transport.spec.ts` |
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
(2026-08-22, run 32562121114) carved the wall on the runner's release
engine and compared it to production — `overall NOISE`, as did the next
two nights; its URL is in `examples/wall/README.md`.

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
- **WP-B the `Solid` kind** — **done 2026-08-20** (`wt/solid`, four
  commits; docs/03 §The sharing model, §No handle cache, §Display
  tessellation; docs/12 §Display cache; docs/14 §Value and geometry
  representations; DECISIONS.md row 16 revised in place):
  `core::Solid` = the canonical bytes in an `Arc<[u8]>` and nothing else
  (header-checked; KindTag 20 over the length-prefixed bytes; goldens for
  the probe box and prism through the new path equal WP-A's raw blake3
  goldens, and their `HashedValue` hashes are blessed); `Solid` in
  `TRANSFORMABLE_KINDS` / `GEOMETRY_KINDS` (the checker admits it into
  `T` ports and display sinks; `Similarity::apply` on a Solid is a red
  node with `SOLID_TRANSFORM_DEFERRED` until WP-C); `StoredValue::Solid`
  appended with `LOG_FORMAT` 2 → 3 and `tests/fixtures/pack-24d558b.bin`
  (a pre-Solid engine's pack, every blob loaded under its golden hash);
  `ScriptError::Unmarshallable { kind: "Solid", .. }` at the Python
  boundary, bare or in a list. **The sharing model**: op-local, LINEAR
  handles — every `occt::Handle` owns its `TShape` graph (a `BinTools`
  read is the deep copy), kernel operations consume their handles
  (booleans raise input tolerances in place; the mesher keeps a finer
  triangulation), results go back to bytes; the process-wide kernel lock
  is retired (no OCCT global is written by the glue's calls;
  `Interface_Static` for STEP is WP-C's to lock); proved by an 8-thread
  rayon test over 13 related values against single-threaded goldens.
  **No handle cache**, on numbers (release, 1,000 parts): a block's
  re-read is 41 µs against a 3.1 ms boolean (1.3 %), the whole box →
  extrude → difference → tessellate chain pays 5 % for re-reading at
  every step (4.54 vs 4.30 ms per part), and on the rayon pool the same
  chain runs 6.4× the serial rate (710 µs wall per part) — the lock would
  have held it at 1×; `examples/solid_bench.rs` is the table. **Display**:
  `cicada_geom::solid` (value level, same signatures in every build —
  `GeomError::KernelUnavailable` without the feature, never a mesh-tier
  fallback); the session's `display::SolidCache` keyed by value hash +
  deflection, bounded (256 MiB), LRU, counters in `/debug/state` →
  `display_cache`; `Deflection::display` = `max(0.02 mm / unit, tol)` and
  `max(0.1 rad, tol_angle)`; the frames stay mesh frames keyed by the
  Solid's hash; `DisplayStats.solids` / `.errors` additive; the summary is
  "Solid, N faces, bbox"; web hue `--kind-solid`. `cargo test --workspace`
  passed with and without `--features occt` at the time (892 / 868
  tests; since WP-C's default flip only cicada-geom has a kernel-free
  world and the server's tests assert the kernel is present — second
  review closure below), and a local flip of `default = ["occt"]`
  checked the whole workspace.
  **Consequence WP-C acts on**: once `box` / `extrude` / … are
  OCCT-backed stdlib nodes the product build needs the kernel, so WP-C
  flips `occt` to a default feature of `cicada-geom` (and revises the
  DECISIONS.md row-16 sentence "the `occt` cargo feature is OFF by
  default and default builds never touch OCCT" in the same commit), and
  every CI job runs `tools/fetch_occt.py` first; until then the
  `occt (ubuntu)` job should add `cargo test --workspace --features occt`
  so the server's Solid display tests run in the kernel world in CI
  (today they run there only on this machine). **Review fixes (same
  day)**: a compile-time `!Sync` assertion on `occt::Handle` (the belt
  behind `canonical_bytes(&self)`); the sharing model proved under the
  SCHEDULER too — `cicada-server/tests/solid_scheduler.rs`, closures
  over `cicada_geom::solid` in a `SolveGraph` with a 48-element `each()`
  fan-out on 8 threads vs 1, hashes equal (the node-level `.cic` form
  follows WP-C's nodes); the review's UNVERIFIED determinism question
  answered with evidence — `canonical_bytes_do_not_depend_on_heap_state_or_thread`
  (a seven-cut, 58-face solid under allocator churn and on 8 threads,
  byte-identical; WP-C reruns the shape on loft/revolve before blessing
  goldens); `Deflection::new` floors at the kernel's own
  (`MIN_LINEAR_DEFLECTION` 1e-7, `MIN_ANGULAR_DEFLECTION` 1e-12 — the
  mesher throws below them; WP-C's `tessellate` node inherits the "Red
  when"); `SolidCache` touches and evictions are O(log n) (a recency
  index, asserted at 2,000 entries without the kernel) and an entry
  larger than the whole budget is served, counted `oversized`, never
  kept; `/debug/state` → `display_cache` and the omitted-when-empty
  `stats.solids`/`stats.errors` are asserted by the session test and
  listed in docs/13; the five transform nodes' `# Panics` (and the
  catalog's "Red when") name the Solid refusal, with a node-level test;
  the Python refusal names `tessellate` as ARRIVING with WP-C, not as a
  node to use. Not changed, on purpose: the handle cache stays unbuilt
  (item 4 of the package — the measured ≤ 5 % gain and the fork-glue
  conditions in docs/03 §No handle cache; an orchestrator-level
  acceptance, recorded here, and re-measured by the second review: the
  reader nodes on the 300-hole bar are 1.6 % of its boolean), and
  display tessellation still runs under the session lock (§Follow-ups;
  closed below). **Second review closure (2026-08-21, after WP-C/WP-D;
  findings 1–8 of the adversarial review):** (1) *a valid Solid could
  vanish from the viewport* — a sphere moved by the kernel transform,
  minus a cylinder through both its poles, meshed with 159 T-junctions
  and the display path refused any unclosed mesh; root cause found and
  fixed at its source: `BRepBuilderAPI_Transform(Copy = true)` leaves a
  pcurve on the SOURCE surface on the degenerate pole edges (the moved
  sphere serialized with two spherical surfaces), the cut carried it and
  the mesher discretized the intersection edge differently per face —
  `transform` now drops pcurves on surfaces no face uses (docs/03 §The
  node-set glue), after which the moved sphere serializes to its
  in-place twin's size and the cut meshes closed with the twin's
  triangle count; AND the policy is split — `tessellate` (the node)
  requires closure, `tessellate_display` reports it, display draws the
  welded mesh either way with `watertight: false` / `unclosed: N` in the
  summary and `stats.warnings` in `/debug/state`; `solid::is_valid`
  (`BRepCheck_Analyzer`) exposed for diagnosis — it called both solids
  valid. (2) *display under the session lock at a fixed 0.02 mm* — the
  display edge now runs on the solve loop's worker pool BEFORE the
  session lock (`Scheduler::map_parallel` over the generation's distinct
  solids; the broadcast only hits), at two TIERS (preview generations
  0.1 mm / 0.3 rad, structural 0.02 mm / 0.1 rad, the release redrawing
  a preview-tier value, tiers recorded per output, a joining client
  restreamed at the tier on screen) with a relative term (1/1000 of the
  solid's largest extent — OCCT's viewer convention); numbers below.
  (3) *blob frames keyed by the Solid's hash while their content
  followed the deflection* — the blob key is now the display mesh's own
  value hash (content-addressed like a `Mesh`; two tiers → two blobs,
  asserted; docs/13). (4) the Python refusal reworded to the present and
  its test now asserts the named node EXISTS (a cli test against the
  registry) and rejects milestone wording. (5) kernel refusals are
  typed: `GeomError::NotOneSolid { operation, found }` ("cut left 2
  solids — a Solid is one body; …"), glue `name: ` prefixes stripped,
  operation labels plain words, and `diagnostic_vocabulary.rs` rejects
  `cicada_` / "shape type" in every seam literal. (6) refusals are
  cached as negative entries (`display_cache.refusals`); docs/12 no
  longer claims a refusal is cheap. (7) the seam goldens route through
  `platform_golden(win64)` like the stdlib's; the three-OS verdict is
  PENDING the first CI run of this branch. (8) the dead kernel-free arms
  in cicada-server's tests are gone (the crate links the kernel
  unconditionally; each test asserts the kernel is present) and the
  Linux job now RUNS `cargo test -p cicada-geom --no-default-features`
  (78 tests) instead of checking it; the ledger's "Scheduler internals"
  row says a new `StoredValue` variant bumps `LOG_FORMAT` (revised in
  place, dated). **Measured (release, dev machine, 2026-08-21):**

  | case | before (review) | after |
  |---|---|---|
  | 02-solids `size` slider, as committed (its `tessellate(deflection=0.01)` export node in the cone) | 42 / 135 ms p50/p95 server | 39.9 / 54.9 ms — the `shell` node alone is 34 ms per tick; display is no longer the cost |
  | 02-solids `size` slider, the display part only (`shell`/`dump` removed from a scratch copy) | — | **5.2 / 9.1 ms server, 5.5 / 9.4 ms client; 300 of 300 ticks at 60 Hz — PASS against 16 / 33** |
  | 300-hole bar, structural load (303 distinct solids) | 5,263 ms generation for a 505 ms solve; `holed` 152,412 triangles | 1,195 ms for a 341 ms solve; `holed` 58,812 triangles (4 mm relative deflection), 300 drills in parallel |
  | 300-hole bar, a preview tick (`pitch` drag) | — | 1,100 ms per tick for a 330 ms solve: `holed` at the preview tier (51,612 triangles) is ~0.7 s of mesher on two 6,300-vertex planar faces — one solid, one kernel call, the remaining cost |
  | wall carve (`--node carved`, fresh cache) | `507f582b…` 3.80 s | `507f582b…` 3.75 s — unchanged |

  The bar's remaining display cost is a single `BRepMesh` call that no
  tier makes cheap; docs/12's costed, cancellable display edge is where
  it goes (§Follow-ups). Left for WP-C from WP-B:
  the kernel-backed transforms (`Similarity::apply`'s Solid arm and the
  five transform nodes' `# Panics`, which then lose the Solid sentence),
  a `tessellate` node over `solid::tessellate` (its "Red when" includes
  the deflection floor), the STEP nodes' `Interface_Static` lock, and —
  if a handle cache is ever wanted — `SetNonDestructive(true)` booleans
  plus clean-before-mesh in the fork's glue (docs/03 records the
  conditions). One ledger wording for Ben: the "Scheduler internals" row
  says `LOG_FORMAT` bumps with a new `LogRecord` variant; WP-B bumped it
  (2 → 3) for a new `StoredValue` (blob codec) variant on the same
  reasoning — an older engine tombstones and recomputes every memo whose
  Solid blob it cannot decode — and docs/12 says so; the row's sentence
  could read "any new record OR value-codec variant".
- **WP-C nodes — done 2026-08-20** (`wt/solid`, eight commits on top of
  WP-B; docs/03 §The node-set glue, docs/08 §7–§9/§11 rows as shipped,
  DECISIONS.md row 16 revised in place). **The default flip**: `occt` is
  a default feature of `cicada-geom` (the product's nodes need the
  kernel); every cargo command runs under `tools/fetch_occt.py`'s env
  (AGENTS.md palette) and every CI job that builds fetches the prebuilt
  first — the dedicated `occt` jobs folded into the standard per-PR and
  nightly matrices, macOS on an rpath, the Linux job keeping the
  kernel-free `--no-default-features` build compiling. **The glue lives
  in cicada-geom**, not the fork: `src/occt/glue.hxx` + `glue.rs`, a
  second cxx bridge compiled by the crate's own `build.rs` with cxx-build
  against the same `DEP_OCCT_ROOT` (the fork is pinned by rev and every
  patch to it is a release of a second repository; these are Cicada's
  own kernel calls and change with the node set) — same rules: every
  function `Result`, the trycatch hook repeated per translation unit, no
  OCCT global written except by the STEP translators, which run under
  `occt::STEP_LOCK`. **The nodes** (all three tests each; every Solid
  node's table test runs in BOTH worlds through `with_kernel` — without
  the kernel the node must be red with the typed refusal; a claim that
  became TRUE only with the review closure below, when the stdlib got a
  kernel-free build of its own): `box`,
  `sphere`, `cylinder`, `cone` (BRepPrimAPI in a plane's frame; a
  world-frame `box` is byte-identical to the seam's `box_at`), `extrude`
  (exact edges for every curve kind: a circle is an exact cylinder, so
  no `segments`; a rectangle's prism equals `extrude_polygon`'s bytes),
  `extrude_to_point`, `loft` (`profiles: [Closed<Curve>]`, `ruled = true`
  — GH "Straight", the wall's and the mesh tier's behaviour; `false` =
  GH "Normal"; sections made compatible first), `revolve` (a `Line`
  curve axis in the profile's plane; touch but never cross; an angle
  domain whose start and sign are one rigid kernel transform), `sweep`
  (Sweep1: MakePipeShell, corrected Frenet, mitred corners) and `pipe`,
  `solid_union` (n-ary, ONE general-fuse pass), `solid_difference` (one
  solid, a cutter list — the `mesh_difference` shape), `solid_intersection`,
  every boolean followed by `UnifySameDomain` (two fused cubes are a
  six-face box) and every result required to be ONE body (disjoint
  unions, splitting/emptying cuts and empty intersections are red — the
  B-rep tier has no empty solid), `volume`, `bounding_box` (exact on
  points/curves/meshes, kernel bounds for solids, in any plane),
  `deconstruct_solid` (edges + vertices + `face_count` — no `Surface`
  kind yet, and the port is named so the future `faces` port does not
  collide), `section` (one closed curve per loop; a circular loop is an
  exact `Circle`), `tessellate` (weld + watertight + Manifold
  acceptance; the deflection floor inherited from WP-B as its "Red
  when"), `export_step` (effectful; AP214 in the document unit; the
  header fixed AND the products renumbered in file order — OCCT numbers
  them from a process-wide counter, measured: a second export in one
  process wrote 'parts 11' — so the same solids give the same bytes,
  proved by a write-twice test) and `import_step` (the first shipped
  `volatile` node: a file on disk is external state; the registry test
  now holds a sanctioned list of one; its example reads `block.step`
  "next to the pipeline" — the example runner stands where a pipeline
  stands and provides the stdlib's committed fixture there, since the
  review closure). **The
  transforms**: `Similarity::apply`'s Solid arm goes through the kernel
  (`solid::transform`), so `move` / `rotate` / `scale` / `orient` /
  `linear_array` accept a `Solid` as `T` with no arm of their own, and
  `mirror(geometry: T, plane: Plane = xy_plane)` — docs/08 §10 tier 1,
  added 2026-08-21 — is the plane reflection (`Similarity::reflection`;
  a mirrored `Solid` is reversed by the kernel, its volume positive, its
  bounds on the far side: asserted at the node in both worlds). **The rename**:
  `mesh_box` / `mesh_sphere` / `mesh_extrude` / `mesh_loft` (Mesh &
  field; gh Mesh Box / Mesh Sphere, and honestly Extrude / Loft for the
  two GH reaches only via Extrude/Loft + Mesh Brep), outputs
  byte-identical — the mesh goldens did not move and the wall's carve is
  still `507f582b5fe4b575cacc71e34346db2f9ab9c25391ce74ff8f9550dc877fa8a2`
  (release, fresh cache: 3.7 s wall cold on the dev machine, 44.6 s CPU
  over 22 workers) — and every consumer migrated (wall.cic, 03/04/06,
  the mesh nodes' examples, three cicada-server test fixtures whose
  stories are mesh stories, the smoke's binding count).
  **Goldens**: `HashedValue` hashes of canonical bytes for
  transcendental-free inputs only (boxes, rectangle prisms, a polyline
  loft, box booleans, a box's section loop and vertex list), blessed via
  run-once on win-64 AFTER `node_set_bytes_do_not_depend_on_heap_state_or_thread`
  (the churn + 8-thread shape WP-B's review asked for) passed on that
  corpus plus the revolve and sweep; they reach the tests through
  `platform_golden`, the one place a second OS adds its arm (row 42);
  curved primitives, revolve, sweep and pipe hash nothing — run-to-run
  identity + analytic volumes (every primitive, sweep and boolean has its
  volume formula as oracle, read by the integrator AND the
  tessellation). `cargo deny check licenses bans sources` green (the
  advisory DB needs network). 507 stdlib tests, 123 in cicada-geom.
- **WP-D consumer — done 2026-08-20**: `examples/07-simple-cad.cic` —
  the bracket (a `box` plate, a `cylinder` boss, an `extrude`d gusset
  4 mm inside the boss so no tangent contact reaches a boolean, one
  `solid_union`, a through-bore and a `linear_array` of mounting holes
  removed by one `solid_difference`, `volume`, a `section` through the
  plate — the holes come back as exact circles — `bounding_box`,
  `export_step` on `--node step`; eight sliders; 55 nodes, 21 ms wall
  cold on a debug build). `web/e2e/simple_cad.spec.ts` opens it in the
  app: every binding green, the bracket's display stats count one Solid
  with triangles and the bounds [0,0,0]–[80,50,28], `bounding_box`
  agreeing, the Solid cache holding entries, frames in the viewport.
  `examples/02-solids.cic` is now the B-rep example (box, sphere,
  `solid_difference`, `volume`, one `tessellate` for the OBJ exporter).
- **Measured (release, fresh caches, 2026-08-20, the dev machine — 22
  rayon workers), against docs/15 §Stage-6 results:**
  - *Wall cold carve*: 3.7 s wall (gate < 10 s; stage 6: 6.5 s) — the
    mesh tier is untouched; faster only because the machine is.
  - *Cheap-cone slider, `02-solids` `size`* (now an OCCT box → sphere →
    `solid_difference` → `volume` → `tessellate` cone; `tools/measure/slider_loop.mjs`,
    300 sends in 5 s at 60 Hz): 122 preview generations (23.8/s, 178
    superseded), server queued+elapsed **p50 42.1 ms / p95 135.4 ms**
    (elapsed p50 25.7, queued p50 10.4), client round-trip **p50 32.6 ms
    / p95 98.0 ms**, longest silence 49 ms, 0 errors — **FAILS the 16/33
    ms bar** (stage 6, mesh tier: 0.5 / 1.4 ms). The cause, isolated:
    the kernel work is within budget — per tick `box` 0.2 ms, the cut
    2.3 ms, `volume` 1.4 ms (reconstruction from bytes is in those
    numbers; no lock exists) — and with the explicit `tessellate` node
    removed the generation still takes 25 ms, because the carved Solid's
    DISPLAY tessellation at the display deflection (0.02 mm / 0.1 rad on
    a 0.75-radius spherical cut: 4,266 triangles) costs **23 ms** in
    `BRepMesh_IncrementalMesh` (the sphere alone at that deflection: 37
    ms / 8,000 triangles; the same solid at 0.1 mm / 0.3 rad: 3 ms /
    532 triangles), and it runs inside the generation on every new value
    (a new Solid is a cache miss by construction). Generations longer
    than the 16.7 ms tick then queue (queued p50 10–24 ms). Follow-up
    named below: a preview-time display deflection (coarse while
    dragging, the 0.02 mm mesh on release), and the display edge moved
    off the session lock and onto the solve loop's workers (docs/12).
  - *Esc during a long boolean* (`tools/measure/esc.mjs`, 20 trials,
    0 missed): a **4,000-element `each()` chain** of `solid_difference`
    (29 s CPU, 1.5 s wall) — time-to-idle client **p50 6.2 ms / p95 31
    ms**, server cancel→idle p50 4.3 / p95 6.3 ms: **PASS** (the cancel
    lands between chunks; the sharing model's lock-free pool is what
    makes the chain 20× parallel). **ONE 1,000-tool `solid_difference`**
    (a 3,000 mm bar minus 1,000 cylinders, one kernel call of 1.4 s) —
    time-to-idle client **p50 1,689 ms / p95 1,790 ms**: **FAILS 250
    ms**. A single kernel call is not interruptible from Rust — the
    token is checked between nodes and chunks, never inside OCCT — so
    Esc waits for the boolean to finish. The doc-12 kernel worker (ops
    the cost model predicts above ~1 s routed to a cancellable
    subprocess, killed on Esc) is the named follow-up; until it lands a
    long B-rep boolean holds the session for its own duration.
- **Done when** (as written, with the verdicts): the wall's cold carve
  re-measured unchanged ✓; the 02-solids slider re-measured — NOT
  unchanged: it now drives the OCCT cone and misses the bar for the
  reason above (recorded, follow-up named); golden hashes for
  transcendental-free solids: committed on win-64 through
  `platform_golden`, the three-OS verdict comes with the first CI run
  of this branch (row 42's per-OS policy applies if they disagree); Esc
  during a long boolean measured and written down, the kernel worker
  named ✓; `cargo deny` green ✓; CATALOG.md regenerated ✓.
- **WP-C/WP-D review closure (2026-08-21; findings 1–8 of the
  adversarial review, five commits on `wt/solid`):** (1) *blocker — a
  pre-flip memo served a Mesh to the Solid-typed `box`*: the flip
  changed `box`'s output kind under an unchanged name, port list and
  `version = 1`, and the memo key `(op, version, tolerance, inputs,
  fan)` says nothing about what an op returns, so a store warmed by any
  earlier engine handed the mesh box back, green (reproduced by the
  review with main's engine and the branch's on one `--cache-dir`).
  `box` / `sphere` / `extrude` / `loft` are at version 2 (old caches
  recompute the four once); `cicada-server/tests/stale_memo.rs` plants
  the pre-flip entry through the real lowering and requires `block` to
  COMPUTE a Solid, with a control that plants the same entry at the
  current version and requires the hit (so the planted key is the
  executor's); and the rule itself is now a test — the conformance suite
  holds every `(name, version)` to its recorded signature and
  key-relevant flags in `crates/cicada-stdlib/tests/signatures.tsv`
  (bless a new row with `CICADA_BLESS_SIGNATURES=1`; the four version-1
  rows were reconstructed from main's catalog.json so the ledger states
  the history the flip crossed). The memo-hit kind check the review also
  offered is NOT built: entries carry hashes only, so it would read every
  output blob on every hit (the warm path is hash-only, docs/12) or add a
  `LogRecord` with the kinds for information the version already
  carries. (2) *major — `tessellate` admitted any deflection the kernel
  admits*: a unit sphere at 1e-7 had 23 GB of mesher state after 25 s
  and never finished, in one uninterruptible kernel call. The node now
  has a budget stated in the part's terms — never finer than
  `TESSELLATE_MAX_FACETS_PER_TURN` = 1000 facets around a full turn at
  the solid's largest extent: angle ≥ 2π/1000, deflection ≥
  (L/2)(1 − cos(π/1000)) ≈ 2.5e-6·L (the default 0.01 admitted up to a
  4 m part; the review's request 50× below the line) — checked between
  one handle's bounds read and its mesh (`solid::tessellate_within_budget`),
  refused typed (`GeomError::TessellationBudget`) with the floors for
  THIS part in the message; at the budget a sphere the size of the part
  is ~10⁶ triangles, the most a deliberate request may cost. Display
  tessellation never reaches it (asserted). (6) the same node dropped
  `uses_tolerance` (it read no tolerance; the slot only invalidated every
  tessellation on a tolerance change) — version 2. (3) *`section` false
  red on a tangent plane*: the seam now tells a tangent contact (the
  plane touching the solid along a line or curve without entering it)
  from a loop edge by probing 100 tolerances either side of the edge in
  the plane — both probes classify alike for a contact, differ for a loop
  edge — drops the contacts (counted in `occt::Section`) and requires
  what remains to close, so an open chain with the solid on one side is
  a typed kernel failure, never a loop; per EDGE, because a graze joined
  onto a loop arrives as one open wire (the plate outline with the bore
  graze as a chord). The tangent cylinder, the box-edge plane, a miss,
  the bore graze and the same plane a hair inside the bore are the tests
  at the seam and at the node; `section` at version 2. (5) *"both
  worlds" was dead code*: the stdlib takes cicada-geom with
  `default-features = false` and forwards its own default-on `occt`
  feature; `cargo test -p cicada-stdlib --no-default-features` is a 9 s
  build that CI's Linux job runs — and the first honest run showed 24
  tests whose Solid fixtures unwrapped kernel constructors before the
  node was reached, now routed through `support::fixture` (the built
  solid with the kernel, a pseudo solid without, so the refusal asserted
  is the NODE's own) and `support::expect_red` (a red in both worlds:
  the kernel-world reason, or the kernel refusal). (4) *the MCP server
  died silently without the loader path*: `.mcp.json.example` carries
  an `env` block (the Windows layout via `${LOCALAPPDATA}`), `tools/
  fetch_occt.py --print-env mcp` writes the registration for any OS with
  the absolute library dir and the right variable, `cicada-cli/tests/
  mcp.rs` asserts the example; AGENTS.md says the path is needed
  wherever the binary is launched from. (8) `import_step`'s example is
  `block.step` next to the pipeline; the example runner mirrors `cicada
  run`'s chdir and provides the stdlib's fixtures there. (7) *three-OS
  goldens and the CI fetch steps*: still unverified by any run — the
  branch has not been pushed; the workflow files parse and every
  building job carries the fetch + env step (read, not run); the
  per-OS arms of `platform_golden` stay single until the matrix speaks
  (row 42). The status row's stale job names are fixed. **Re-measured
  after the closure (release, fresh caches, the dev machine):** the
  wall's carve hash and the 02-solids slider numbers are in the report
  of the closure run and unchanged in kind — the budget adds one bounds
  read (microseconds) to `tessellate`, the contact test runs only on a
  section wire that did not close.

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
`examples/08-orbit.cic` in the same slice (06 went to the lists
example, 07 to item 3's bracket). Clock follows via `volatile`.
**Done when**: the second pass of a loop is 100 % `cached` with an
identical NodeKey set; "previews never write the file" holds under
playback (test); a headless run yields frame 0.
- **Shipped (engine half, 2026-08-20, `wt/transport`, three commits)**:
  (1) the nodes — `cycle(period = 4, frames = 120, frame = 0)` =
  `(frame mod frames) / frames`, `clock(speed = 1, t = 0)` = `t × speed`
  and volatile; `#[port(transport_driven = frame | time)]` →
  `PortSpec::transport_driven`, `catalog.json` `"transport_driven"`,
  the CATALOG.md tag, `cicada mcp`'s `PortDoc`; the macro refuses a
  transport-driven port without a default (its headless value).
  (2) the transport — the lowering is the injection point
  (`lower_with_playhead` / `lower_partial_with_playhead`; every session
  lowering passes the playhead, `cicada run` passes none); anchored
  playhead state on the session's injectable op clock; the five
  intents, writer-only by decision (shared state, the lease arbitrates —
  docs/13 §Animation transport says why), never an op or a delta; Esc
  and the last client leaving pause it; the ticker at 60 Hz submits
  `JobKind::Transport` jobs to the one-slot loop only when the driven
  values moved; a live drag's thumb rides along; a wired `frames` /
  `period` is the one red the transport adds; `TransportView` in every
  snapshot + the `transport` broadcast + `/debug/state.transport` +
  the `transport` timing kind. (3) `examples/08-orbit.cic` and the docs.
  The three "done when"s are session tests
  (`a_second_pass_of_the_loop_is_entirely_cached_with_identical_keys`
  rebuilds the NodeKeys exactly as the executor does and compares them
  frame for frame; `playback_never_writes_the_file` — 200 frames, bytes,
  hash and mtime unchanged, no op, no delta; the lowering test solves
  the headless graph to frame 0 / t 0) plus the http e2e driving
  play/pause over the real socket. Measured on the orbit example
  (debug build, 15 nodes, 30 fps loop): first pass 120 generations,
  1,190 computed / 610 cached, p50 1.5 ms; second pass 120 generations,
  0 computed / 1,800 cached, p50 0.43 ms. Browser evidence with the
  EXISTING SPA (it knows nothing of the transport and renders the
  transport generations' frames like any other): `transport_play` sent
  through `window.__cicada.send`, viewport screenshots at rest, at
  frame 30 and at frame 63 all differ, the planet and moon where the
  angle says; the page received 131 frames over the two seconds.
- **Shipped (the web half, 2026-08-20, `wt/transport`)**: the play bar
  (play/pause, the frame scrubber over the primary loop, speed, reset),
  Space toggles when no text field has focus, the `transport_driven`
  ports hidden on the canvas and in the inspector (the catalog says
  which; each driven port carries its OWN loop so the inspector shows the
  frame IT is fed, never the primary loop's — a second `cycle` loops
  inside at its own rate — additive `DrivenView.loop`), the wire-target
  rule moved to the SERVER (`wire_verdict` — `probe_wire` answers
  `blocked`, `connect` refuses; a wire the text carries is kept, drawn
  and removable, never hidden), `driven` rendered on the time nodes, the
  playhead extrapolated between `transport` broadcasts, observers
  read-only, and `web/e2e/transport.spec.ts` on the orbit and a
  two-cycle + clock pipeline (the `/debug/state` oracle: the transport
  kind, per-port loops, the seek exactness, the refused wire). Engine
  touched under the same package: `transport_seek` now lands the first
  representable playhead INSIDE the frame (`Playhead::at_frame`; a bare
  nominal seek painted frames 31/62/65 one short), and each `DrivenPort`
  carries its loop into the view. Candidates beyond the slice (not done):
  the `transport` view on the status bar's generation line; a `transport`
  field in `GET /api/project`.
- **Review fixes (2026-08-21, `wt/transport`)**: the ticker ran at ~33 Hz
  on Windows (`Condvar::wait_timeout`'s 15.6 ms quantum), so a 60 fps
  loop warmed 133 of 240 frames on its first pass and the "second pass
  100 % cached" DoD held only for loops ≤ ~30 fps — the ticks now lie on
  an absolute grid (`anchor + N / 60 s`) walked with the high-resolution
  `thread::sleep`, re-anchored by every control; measured with the new
  `tools/measure/transport_loop.mjs --expect warm`: a 240-frame / 4 s loop
  at 60.0 generations/s, gap p50 16.66 ms, first pass every frame (239
  computed), second pass 0 computed (docs/13 §Latency targets carries the
  number). A real-ticker session test (`the_ticker_thread_paints_frames_on
  _its_own`) drives the thread itself; the lost wake-up at Play is closed
  (the playing flag is stored and notified under the gate the paused wait
  reads it under); Esc pauses BEFORE cancelling, under the lock the ticker
  submits under, and a ticker tick that finds the transport paused
  submits nothing; the frame dedupe has a sub-frame test (a dedupe on the
  raw time passed every test before) and its `clock` mirror. A refused
  control broadcasts nothing (docs/13 said "refused or not"; the test
  pins the truth).

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

## Usage findings 2026-08-24 (Ben's first user test) and wave 4

Ben user-tested the app on 2026-08-24 and pasted the findings below; each
carries a verdict and the wave-4 package that owns it. The order Ben set:
these first, then the rest of wave 4 (item 5 scrub caching, catalog C2,
the follow-ups).

| # | Finding (Ben's words, condensed) | Verdict | Package |
|---|---|---|---|
| U1 | Which launch commands run every time? `fetch_occt.py` takes long. | Measured: `fetch_occt.py --print-env` is 0.2 s (the warm path verifies 88 libraries by size); what takes minutes is the first fetch and any `cargo run` that rebuilds. Per launch the ONLY need is the kernel's loader path, and the launcher below removes even that (the run-time libraries beside the binary on Windows, the rpath on macOS). | L1–L2 |
| U2 | A bundled Windows/macOS executable: a dedicated app window (a browser wrapper is fine) plus a terminal that builds and runs the server. | Build. `cicada app` (serve + the system browser in `--app` window mode), `tools/launch/` (the dev launcher: a terminal that builds if needed and runs; the bundle: a folder with the binary, its run-time libraries and a double-clickable `Cicada.cmd` / `Cicada.app`). Tauri stays out of v0.1 (DECISIONS). | L1–L3 |
| U3 | Run the server without a file; File → Open picks the `.cic`; close and open another in the same session. | Build. `cicada serve` / `app` without a path serves the user's home directory as the root and opens nothing; `GET /api/files` lists directories and pipelines under the root (never outside it); the app gets File → Open… / Recent / Close and a landing picker; switching pipelines reuses the server's per-pipeline sessions. | O1–O2 |
| U4 | Grid lines ~50 % less visible, closer to the background. | Build (one token pair per theme). | B1 |
| U5 | A small XYZ gimbal indicator in the viewport's upper-left. | Build. | B1 |
| U6 | PCB traces: minimum corner radius ~1 unit; parallel segments never overlap — lanes with ~¼ unit clearance. | Build: a trace router of our own replacing React Flow's smooth-step path. | B2 |
| U7 | Port value previews need too much zoom. | Build: one LOD tier earlier (docs/16 table revised). | B1 |
| U8 | A preview pop-out button: a second synced window for a second monitor. | Build: `?view=viewport` (the SPA as a viewport-only observer) opened by a viewport button. | O3 |
| U9 | No way to type a primitive input (e.g. `construct_domain`'s `end = 40.0`) on a placed node; any keyboard-typeable unconnected input should be directly editable. | Build: the server already accepts `set_param` on ANY node's port (`apply_param`) and the view-model already carries each input's literal (`InputView.literal`) — the gap is the UI: inline editors on unconnected literal-typed ports, on the canvas and in the inspector. | B3 |
| U10 | Slider shortcuts like Grasshopper's `1<20` and `0.0<0.5<1.0`. | Build: search-to-place parses them; the decimals typed set the step. | B4 |
| U11 | Sliders collapse to a single-unit-tall node (GH-like); refuse when min/max/step are wired. | Build: the sidecar's existing `collapsed` override, a `set_collapsed` intent (an op), the collapsed slider view. | B4 |
| U12 | Repo is public now → a big WIP disclaimer on the README. | **Done 2026-08-24.** | — |
| U13 | Public → never leak machine contents, passwords, anything identifiable; say so where agents read. | **Done 2026-08-24**: the AGENTS.md working rule, and a scrub of the four tracked files that carried the owner's home path (the OCCT probe memo, the probe crate's patch path, a conda helper's default cache dir, the wall layout tool's default wall path — now `CICADA_WALL_REPO`, required). The git HISTORY still holds that username inside those paths (no secrets anywhere); rewriting it is Ben's call and not recommended. | — |
| U14 | Why does the wall's carve take so much longer now — solids instead of meshes? | **Measured 2026-08-24**, release engine, fresh cache: cold 3.93 s (3.7–3.9 s on 08-20/21), warm 0.15 ms; the debug engine 6.5 s. The wall is still entirely mesh-tier (`mesh_loft`, `mesh_difference`, `text_solids`, …); no kernel change touched it. What reads as slower: `cargo run` builds DEBUG (6.5 s, after a rebuild); every engine whose node versions moved (14 bumps in the count-guard work, the `mesh_*` renames) recomputes the wall once, cold; and the app's first paint adds the 368 MB display encode and the browser's decode on top of the carve (13.8 s to open at 2 threads in the heavy spec). The launcher always runs release. | — |

### Wave 4 — work packages (contracts frozen 2026-08-24)

Three worktrees (`%LOCALAPPDATA%\cicada-wt\<name>`, private target dirs,
`CARGO_INCREMENTAL=0`), each package implement → adversarial review →
fix, committed on the branch, never pushed; merge order open → launch →
canvas; then item 5 / C2 / the follow-ups as the second half of the wave.

**Track O — `wt/open` (server http + cli + web shell).**
- **O1 — the root and the file list.** `cicada serve [path]` / `cicada
  app [path]`: no path → root = the user's home directory, nothing
  opened; a directory → root = it; a `.cic` → root = its parent, that
  file opened (`?pipeline=` as today — pipelines are root-relative).
  `GET /api/files?dir=<root-relative>` (token-gated) → `{root: <display
  name>, dir, parent: string|null, entries: [{name, kind: "dir" |
  "pipeline", modified_ms}]}`: directories (no dot-directories, no
  `node_modules` / `target`) and `*.cic` files, directories first,
  case-insensitive name order; `dir` is normalised and every escape —
  `..`, an absolute path, a symlink whose canonical path leaves the root
  — is `400 path_not_allowed`; an unreadable directory is `403 io_error`;
  documented in docs/13 §HTTP surface; route tests for the shape and for
  every escape. The server still binds 127.0.0.1 only, and the list
  reveals nothing above the root.
- **O2 — File → Open / Recent / Close.** The top bar gains a File menu:
  Open… (a dialog over `/api/files` with breadcrumbs, directories and
  pipelines, keyboard navigation; Enter / double-click opens), Recent
  (the last 10 root-relative pipelines this origin opened,
  `localStorage`), Close (back to the landing picker). Landing: a page
  with `?token=` but no `?pipeline=` IS the picker. Opening switches the
  `pipeline` URL parameter and (re)connects the socket to that
  pipeline's session — the server's sessions are per pipeline already;
  the store is reset by the join's snapshot; `history.pushState`, so Back
  returns to the previous file. docs/16 §Application layout;
  `web/e2e/files.spec.ts` (the scratch `examples/` tree lists; opening
  `02-solids.cic` then `06-lists.cic` shows each graph; Recent holds
  both; Close shows the picker).
- **O3 — the pop-out viewport.** A viewport header button opens
  `window.open(<same URL> + "&view=viewport", "cicada-viewport")`; with
  `view=viewport` the SPA renders the viewport alone (no canvas, panels
  or ribbon), connected as an observer that never takes the writer lease
  (an additive `role=observer` request honoured by the server's join — a
  second window must not steal the first's lease, even across a
  reconnect). Same pipeline, same display set, its own camera (camera
  sync is explicitly not in this slice). docs/13 (the join hint),
  docs/16 §Viewport conventions; a Playwright spec: the pop-out shows
  the geometry and stays read-only while the main window keeps writing.

**Track L — `wt/launch` (cli + tools + docs).**
- **L1 — `cicada app [path]`.** = `serve` + opens the app window: a
  Chromium-based browser in `--app=<url>` mode when one is found
  (Windows: Edge then Chrome via the registry's App Paths or the usual
  Program Files dirs; macOS: `open -na "Google Chrome" --args
  --app=<url>`, then Edge; Linux: `xdg-open`), else the default browser
  on the plain URL; `--no-browser`; the URL is printed either way;
  Ctrl-C stops the server. The terminal it runs in is the server
  console. Browser discovery is a pure function over a probed
  environment, unit-tested.
- **L2 — no loader path at launch.** `tools/fetch_occt.py --bundle
  <dir>` copies the kernel's run-time library closure (the set
  `--check-closure` verifies) beside a `cicada` binary so Windows finds
  them without `PATH` (the exe's directory is searched first); a macOS
  binary already carries the rpath its build env set, and the bundle
  rewrites it to `@executable_path/lib` with `install_name_tool`;
  idempotent and verified by size like the prefix. AGENTS.md's palette
  states the rule: the env is for BUILDING and for dev shells; a bundled
  binary needs none.
- **L3 — launchers and the bundle.** `tools/launch/Cicada.cmd` (Windows)
  and `tools/launch/Cicada.command` (macOS): a visible terminal that (1)
  builds `cicada` in release with the SPA embedded when it is missing or
  stale (`npm run build` + `cargo build --release -p cicada-cli
  --features embed`, the OCCT env and cmake found the way AGENTS.md
  says, every failure printed and the window kept open), (2) bundles the
  runtime beside it (L2), (3) runs `cicada app` with no arguments — the
  home-root picker (O1). `python tools/launch/bundle.py --out dist/`
  produces the redistributable folder — the binary, its libraries,
  `Cicada.cmd` / `Cicada.app` (a minimal `Contents/Info.plist` plus a
  launcher script) and a README — from an existing release build; CI's
  macOS and Windows jobs run `bundle.py --check` on their built binary
  (the closure is present; the binary answers `--help` from inside the
  bundle with no env). README "Run it" section; AGENTS.md palette row.
  The process-level smoke: the bundle's `cicada app --no-browser`
  answers `/health`.

**Track B — `wt/canvas` (web canvas + viewport + docs/16).** Four
packages in sequence, each reviewed.
- **B1 — visuals + LOD.** The grid tokens move halfway toward the
  background (both themes; docs/16 §Theme records the values); a gimbal
  — the X/Y/Z triad in the viewport's upper-left, following the camera
  each frame, axis colours per docs/16 §Viewport conventions,
  non-interactive in this slice; output value summaries appear one tier
  earlier (`near`, zoom ≥ 0.65, instead of `closest`), the docs/16 LOD
  table revised, `grid.test.ts` pinned. *Implemented 2026-08-24; three
  things the contract did not foresee, settled the small way: docs/16
  §Viewport conventions recorded no axis colours, so B1 defines them as
  theme tokens (`--axis-x/y/z`, X red · Y green · Z blue, shared by the
  ground triad); the toolbar overlay already occupies the corner, so the
  gimbal sits under it (56 px down); and the tier gate lived in TWO
  places — `CicadaNode.tsx` renders the summaries, `Canvas.tsx` fetches
  them with `inspect` — so both now read one `showsPortValues` rule.*
- **B2 — traces.** A router of our own (`web/src/canvas/trace.ts`, pure,
  unit-tested) replaces `getSmoothStepPath` in trace mode: orthogonal
  runs with 45° corners whose radius is ≥ 1 grid unit (`hello.unitPx`),
  and lanes — edges whose horizontal or vertical runs share a channel are
  offset from each other by ¼ unit so no two parallel segments overlap,
  assigned deterministically (by the sorted source/target positions,
  then the edge id) so the layout never flickers across re-renders;
  stroke widths and colours unchanged; the connection line
  (`ConnectionLine.tsx`) uses the same path. docs/16 §Canvas conventions.
  *Implemented 2026-08-24; what the contract did not foresee, settled the
  small way and recorded in docs/16: a "45° corner of radius ≥ 1 unit" is
  the PCB mitre — a 90° turn cut at 45° with legs of one unit, the bends
  sharp (`stroke-linejoin: round`); a wire's stubs are pinned to their
  port's row (it must reach its handle), so the no-overlap rule governs
  the free runs — wires out of ONE port share their stub as a trunk —
  and a long forward wire (> 6 units) is a stair whose long run is a
  free, laned channel rather than a stub along a row of nodes; the
  auto-layout puts adjacent layers two units apart, where full legs and
  lanes cannot both fit, so there the legs shrink toward ½ unit before
  runs are let coincide (U6 ranks them: radius "~1 unit", overlap
  "never"); a U-turn needs runs of three legs, not two, or its two
  same-sense cuts meet at 90°; and the lanes are assigned on the row
  model of the graph plus the canvas's live node positions, corrected by
  React Flow's measured handle geometry, while each edge draws from its
  measured handles — the router clamps an assigned channel into what its
  own endpoints allow. Review closed 2026-08-24: the first router
  stacked five of the wall's wires on one lane — its busiest three-unit
  gap carries 22 vertical runs, 8 deep, on seven lines a Z may take; the
  source-row order scattered short runs over the lines before the long
  ones arrived, and the saturation fallback dropped every loser on the
  natural line — so the vertical runs are now solved a column at a time
  by a depth-first search (top-down, natural-first, backtracking only
  where the greedy fails) under the column constraint AND the stub
  constraint (a stub's drawn length is its line's doing, and two ports
  on one row between adjacent columns must not run into each other);
  collapses are measured on the drawing and reported
  (`data-trace-collapsed`, `data-trace-yield`), the wall is a committed
  fixture of the unit test (`web/src/canvas/fixtures/wallTraceWires.ts`)
  and has its own Playwright spec (`wall_traces.spec.ts`, run last:
  opening the wall starts its carve), and both oracles — one module,
  `web/e2e/traceOracle.ts` — measure the cuts at a wire's ends. Still
  open: obstacle awareness (§Follow-ups).*
- **B3 — typed literals on unconnected inputs.** Every input port that is
  not wired and whose type takes a literal (`Number`, `Integer`, `Text`,
  `Boolean` and their `?` forms; never a transport-driven port) shows
  its value as an editable chip on the canvas node row and in the
  inspector's Node tab — the kwarg's literal from `InputView.literal`,
  or the catalog default greyed when the text carries none; double-click
  (canvas) / click (inspector) → an input; Enter commits `set_param
  {node, port, value}` spelled by `paramValueText`; Esc cancels; a wired
  port shows no editor. If the writer's `set_param` cannot ADD a kwarg
  the call lacks, that is a `cicada-lang` change first (skill
  `dialect-change`, fixtures both ways). `web/e2e`: place
  `construct_domain`, type `40` into `end`, the text reads `end=40.0`
  and the node is green.
  *Implemented 2026-08-24 (`web/src/canvas/LiteralChip.tsx` +
  `literalFace.ts`, the canvas row and the inspector row, vitest on
  both surfaces, `web/e2e/literals.spec.ts` — U9 verbatim plus a Text,
  an Integer and a Boolean port; docs/16, docs/10, docs/13). Verified
  first: the writer could already ADD a kwarg a call lacks
  (`set_kwarg` appends into `fn()`), so no grammar change; what it
  lacked was the ORDER — `set_param` appended where a wire inserts in
  spec order — so `writer::set_param` now takes the spec order
  (`apply_param` passes it; fixture `gestures/set_param_insert`) and a
  typed `start` lands before an earlier-typed `end`. What the contract
  did not foresee, settled the small way: (1) the server parses its
  own default rendering into `InputView.default_value` (additive) —
  the macro spells a Boolean default `true`, the chip must say `True`
  and never re-derive the catalog's spelling client-side; (2) the
  chip is ONE mechanism for a present literal, a default and an empty
  required slot, so it replaces the spike's always-open inline inputs
  on present literals (the two specs that typed into them updated:
  `git.spec.ts`, `disable.spec.ts` unchanged in meaning); (3) leaving
  the field commits like Enter — a click elsewhere after typing must
  not discard — and an unchanged value writes nothing (no no-op op);
  (4) the typed editor streams no `param_preview`: Enter is the one
  write, so Esc leaves no preview behind (the sliders keep theirs);
  (5) an unspellable value (`2.5` on an Integer port) is a warning
  notice, an empty number field a cancel; a Boolean's editor is a
  checkbox (Space toggles, Enter commits); (6) ports of other bases
  (`T`, `Any`) with a present scalar literal keep their chip — the
  spike's rule, unchanged; a `[Number]` list port has none.
  Review closure, same day: the number editor was a browser
  `type="number"` input, which sanitises its value before the rule sees
  it — `3,5` was WRITTEN as `35.0`, `1/2` as `12.0`, `abc` cancelled
  without a word — so the documented refusal never fired on a Number
  port; it is a plain text field now (`inputMode="decimal"`) and
  `spellEdit` accepts exactly the dialect's number grammar (`0x10`,
  `1_000`, `Infinity` refused, never JavaScript's reading of them). Two
  minors with it: (7) "unchanged writes nothing" is judged by VALUE
  (`isNoEdit`) — Enter over `start=0` on a Number port no longer writes
  a spelling-only `0.0`; (8) `writer::set_param` rewrites a present
  literal INSIDE its `each(…)` (fixture `gestures/set_param_lifted`), so
  a chip on a lifted literal no longer drops the lift. Vitest, the
  gestures + session tests and `literals.spec.ts` carry each.*
- **B4 — sliders.** (a) Search-to-place parses `A<B` and `A<B<C` (GH's
  grammar; negatives allowed): min A, max B (or C), value A (or B); step
  = 10^-(the most decimals typed), integers → step 1 and an `Integer`
  slider; `min < max` and `min ≤ value ≤ max` or a notice; the row
  previews the slider it will make; placed as ONE op (undo removes it
  whole). (b) Collapsed sliders: the sidecar's `collapsed` override
  reaches the view-model (`NodeView.collapsed`) and a `set_collapsed
  {node, collapsed}` intent writes it as an op (moves are ops; so is
  this); a collapsed slider is one grid unit tall — name, track and
  value on one row, GH-like — refused with a notice while any of `min`
  / `max` / `step` is wired; toggled from the node's context menu and
  the inspector. docs/16; `search.spec.ts` and a slider spec.

Merge: open → launch → canvas (the two CLI tracks both touch `main.rs`
and AGENTS.md; canvas and open both touch `Viewport.tsx`'s header —
small, by hand). Then the verify-change loop on main, the wall hash
unchanged, and the push.

## Follow-ups (found by the v0.1 reviews and measurements; scheduled, not yet placed)

- **Obstacle-aware trace channels (B2 review, 2026-08-24)** — the trace
  router knows no obstacles: a stair's long run along a row, or a Z's
  channel, may cross a node's face (the review counted 39 such crossings
  by 24 of the wall's 70 wires; 20 on 07-simple-cad), and a channel that
  runs along a row a node sits on coincides with that node's port stub —
  the one coincidence the lanes cannot route around. The fix is to treat
  every node box as occupied extents on both lattices (rows and columns)
  so a channel detours around a node on its row as it already does
  around another wire's run; out of the B2 contract, recorded in docs/16
  §Canvas conventions as the router's limitation.

- **The first Nightlies (2026-08-22..24)** — **read and fixed
  2026-08-24**. Three nights red on two jobs with the engine unchanged;
  the `wall corpus end-to-end` job passed `overall NOISE` every night
  and the 3-OS matrix's tests were green. (1) *macOS clippy*:
  `platform_golden`'s first per-OS arm (5a46e84) was a one-armed `match`
  under `cfg(target_os = "macos")` — `clippy::single_match` — and the
  per-PR clippy runs on Linux, where that body does not exist; the arms
  are now a `cfg`-selected table read by one shared lookup (AGENTS.md
  rule: OS-specific behaviour is data, never `cfg`-gated code). (2)
  *Playwright heavy (wall)*: the job drove the wall on the DEBUG engine —
  98–114 s per cold carve on the runner — and the spec's 15 s waits
  became coin flips (08-22/23 the pending chip clearing on release,
  08-24 the writer's hint with an observer streaming); the job now
  builds and runs release, the profile every docs/15 and docs/17 number
  was taken at. Two spec fixes ride along: `retries: 0` (the release
  writes `deboss` and `previews_deferred` is a cold-start precondition,
  and Playwright restarts no server between attempts — CI's one retry
  could never pass and only masked attempt 1 behind "Expected: 0,
  Received: 17"), and the observer page in its own browser context (its
  368 MB software-GL decode cannot stall the writer's main thread; a
  real observer is another browser). Measured locally on the release
  engine (`--threads 2`, 24-core dev machine): open 13.8 s, estimate
  12.8 s, 9 ticks withheld, the release solved once in 14.2 s, writer
  hint 1.5 s / observer hint 7.5 s after the grab (23 of 26 frames in),
  28 more ticks withheld across the no-write releases — 1.4 min, green.
  **The release-engine Nightly (32763474657, 2026-08-24) still failed**,
  at a different line: after the release, the spec's `expect.poll` on
  the store timed out with NO received value — the shape of a
  `page.evaluate` that never got a turn in 15 s (Playwright evaluates
  the generator outside its retry, so a thrown error or a non-null value
  would have been printed). The page, not the protocol: at wall scale
  the writer page redraws ~13 M triangles in software GL on every state
  change, and pinned to 4 cores locally (engine + node + Chromium) the
  writer's own hint took 6 s against 1.5 s unpinned while everything
  passed — a runner's cores are slower again. The spec now keeps two
  oracles apart: the ORDER of intents and answers is asserted off the
  page's own WebSocket frames (tapped, stamped, written to the test's
  output dir, printed into the log on failure — the runner's artifacts
  do not upload), and every wait on the page gets one bound
  (`PAGE_BOUND_MS`, 60 s) — a sanity net, never a clock. Not reproduced
  locally in five runs (unpinned; the engine on 2 cores; the whole tree
  on 4 cores, three times), and the instrumented 4-core run's wire puts
  numbers on the split: tick → `preview_policy` 1–17 ms, `set_param` →
  `delta` 16 ms, `end_drag` → `drag_ended` 1 ms, no tick after the write
  — while the page took 4.46 s from the grab to its FIRST tick (the
  drag's own event handling under a software-GL viewport) of a 4.66 s
  hint; the next green Nightly is the evidence. Also seen
  the same day: `playback_never_writes_the_file` flaked on macOS twice
  (the play's frame-0 generation raced the first tick's on the one-slot
  queue — the test now waits after play, 97dced0), and **the account's
  Actions budget ran out**: from 18:49 UTC every job failed at start
  with "The job was not started because … your spending limit needs to
  be increased" — Cicada is private, and its 30-day usage tallies to
  ~1,840 billed minutes (Linux 349, Windows 221 × 2, macOS 105 × 10)
  against the free plan's 2,000; no live artifacts exist in any of Ben's
  repos, so the earlier "artifact storage quota" message was the same
  budget speaking. Ben's call: raise the limit, make the repo public
  (Actions are free there), or thin the per-PR matrix (macOS is 57 % of
  the bill at 10×; the Nightly matrix already lints all three OSes).
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
  the parked-restream join (hydrated, intents answered, a control text
  asked for after the tick's repaint still precedes it, the superseded
  output not resent); a departed client's restream encoding nothing
  more; the lane assignment of the two display-plane texts; the HTTP
  e2e's join order — all on a permit-paced recording sink, no sleeps,
  and every test that asserts an order attached through
  `attach_client`, the function `client_loop` uses (the review's
  2026-08-21 finding: with the tests' own channels, `attach_client`
  with both lanes MERGED passed everything — now it fails two). The
  review's second round also narrowed two restream costs: the pick
  table's mutex is held for one up-front ask of an output's ids
  (`display::PickIds`), never across its encode — it used to be, and
  the live path takes that mutex under the session lock, so a join
  could stall every intent for one 94 MB encode; and a client that
  leaves mid-restream stops costing before the next output's load, not
  after its encode. Measured with `tools/measure/lanes.mjs` (the wire,
  no browser; the "before" engine `24d558b`'s, sha256 `39b1c29f…`;
  fresh runs 2026-08-21 on the final shape): a tick at the observer's
  snapshot reaches it after 368 MB / 294–331 ms with one queue, behind
  no frame / 1.3–1.4 ms with the lanes; socket open → `hello`
  2,938–3,074 ms → 5.6–6.2 ms; a tick 50 ms into a join is answered in
  3,160–3,348 ms → 1.3 ms, behind the 19–20 small frames encoded so
  far. **The app-side number is not a before/after of the lanes.** The
  heavy spec (`compute_on_release.spec.ts`, headless Chromium,
  software GL) logs the observer's `preview_policy` latency after the
  writer's grab, under a 60 s sanity bound, as a diagnostic of the
  PAGE: it is set by where the page's frame handling stands at the
  grab, which the spec does not control (its observer setup takes about
  as long as the debug engine's ~3 s restream), and the one-queue
  engine can post the better number — reproduced 2026-08-21: `24d558b`
  192 ms with 26 of 26 frames already handled at the grab, the lanes
  7,284 ms with 23 in (the 2026-08-20 paired runs, 21.0 s → 5.9/11.7 s,
  carried the same confound: 14 vs 24/21 frames in at the grab, and
  are withdrawn as evidence). The residual is the page's own message
  queue: the browser takes the whole restream in faster than it
  handles frames, so a text sent once the server has written them is
  legitimately last on the wire — no socket order can fix that, and
  the page cannot be the socket's oracle. Whether the page's seconds
  per 27–94 MB frame are software-GL renders or decode/upload is
  unmeasured; "a GPU browser pays milliseconds" was a hypothesis, not
  evidence. The client's ledger now empties on EVERY `display_reset`
  (counted), not on a change of its generation — the table's max can
  repeat after an output vanished, and a reconnect's reset then kept
  the vanished output painted (`frameBus.test.ts`). Definition of done
  as accepted: the cadence is reached on the socket (a text behind at
  most the frame in flight) and NOT on the page at wall scale; invariant
  (a) of the work order — "a `display_reset` overtaking older frames
  makes the client drop them" — was refuted and replaced by the
  per-output rule (docs/13 point 4). Next, named, not scheduled: frame
  handling off the main thread (decode in a worker → typed arrays to
  the scene), the one change that lets the queue drain at memcpy speed;
  chunked/element-range frames — the one frame in flight is 94 MB on
  the wall, seconds by itself off loopback; a per-output latest-wins
  display queue for display-vs-display blocking; an end-to-end
  discriminator would need the restream throttled on the wire for the
  observer (CDP network conditions); the live `emit_frames` still
  encodes under the session lock (changed outputs only).
- **A count/allocation guard for every count-taking node** — **done
  2026-08-20** (`wt/hardening`, three commits, plus the 2026-08-21
  re-audit that found every node accounted for and added the one missing
  boundary case — `series` at 2^22 builds, 2^22 + 1 is red; docs/08 rule
  7 is the contract, DECISIONS.md row of 2026-08-21 the ledger record;
  the review's fix round of 2026-08-21 is recorded at the end of this
  entry). A count literal or an Integer wire into `count` (a slider is a
  `Number`, and the checker widens Integer → Number only — the
  inspector's set-param, `apply_text` and `length`'s Integer output are
  the vectors) could ask for a capacity the
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
  band let a count literal reach the allocator-failure abort on an 8–16 GB
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
  forbid — recorded as the one assertion not made. **The review's fix
  round (2026-08-21)** closed what the second adversarial pass found.
  (1) *The ceiling is charged on what a node EMITS, all outputs
  together* — the justification is per slot the value model hashes, and
  `divide_curve` emits three lists: charged per port it admitted
  3 × 2^22 slots and measured 5,332 MiB / 1,172 MB of cache at
  `count = 2^22` (2.15× the series figure the ceiling is justified by,
  the allocator-failure abort on the 8 GB class the guard exists to
  prevent); it now charges `3 × samples` (`count + 1` open, `count`
  closed — the kernel's own rule, read from `Curve::is_closed`) through
  `checked_size`, so `count = 1398100` is the last allowed on an open
  curve and its at-cap footprint is 2,039 MiB / 410 MB against `series`
  at 2,482 MiB / 365 MB in the same run (branch debug engine; on the
  constant). The fence-post `range` charges the `steps + 1` it emits the
  same way (`steps = 4194303` is the last allowed; the port-only check
  admitted 2^22 + 1 slots). Both take their floor through the shared
  `checked_floor` and name the port and its value in the red text
  (`range: values at steps=4194304 (steps + 1) would be 4194305 — above
  …`). Both went to **version 3**: their version-2 band had been
  admitted by the branch's engines (docs/12: any behaviour change). (2)
  *The `…refused_not_allocated` name is held to what it claims*: the
  review moved the guard after the allocation in `random` and
  `duplicate` and their tests so named still passed (11/11) — nine
  guarded files had that assertion only at cap + 1 (32–64 MiB,
  buildable). Every guarded file now carries the 10^11-shaped case with
  the exact message (`random`, `range`, `repeat`, `duplicate`,
  `pad_last`, `divide_curve` at `count = 10^11`; `extrude`, `loft`,
  `voronoi` at `segments = 10^11` on the tessellated input), the cap + 1
  cases are renamed `…one_past_the_ceiling_is_red` (they pin the
  boundary and the message, not the order), and
  `tests/conformance.rs` holds the rule: a file that calls a guard must
  hold a `…refused_not_allocated` test, and every test so named must
  carry a literal ≥ 10^10 and never the cap constant — re-running the
  review's `random` mutation now aborts the test binary ("memory
  allocation of 800000000000 bytes failed"), and the rule rejects a
  cap-constant body with both halves of the reason. (3) The ledger row.
  (4) The measurements are headless `cicada run` numbers — what `cicada
  serve` adds by encoding a 1 GiB list of meshes into display frames is
  unmeasured and belongs with the frame follow-up above; said on the
  constant. Accepted as-is from the review: the helper shape
  (`checked_count(node, port, value, least, bytes_per_slot)` → `usize`,
  panic-based — stdlib has no `NodeError`, the scheduler turns the panic
  red, and `run_e2e` matches the text); the two files outside the stated
  stdlib scope (`cicada-geom/src/text.rs`'s `Font::outline_spans`, the
  sanctioned `stdlib → geom` edge, and the `run_e2e` message).
- **Tessellation `segments` bound memory, not time** (found by the
  guard review, verified 2026-08-20): `extrude` of a circle at 50k
  segments takes 2.0 s, 100k 8.7 s, 200k 37 s (release; the cap ear clip
  is O(n²)) — everything under the 2^22 ceiling is admitted and a count
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
  `resolve_targets` → `lower` → `Scheduler::solve` functions `cicada run`
  prints over, with a fresh `DiskStore` per example (nothing comes from an
  earlier run, so a node that broke cannot hide behind yesterday's memo)
  and no `--node` (every non-effectful leaf; the exporters' inputs are the
  targets and the exporters themselves are never lowered — `lower` takes
  the upstream closure of the leaves — so they have no outcome at all; the
  test pins that no effectful binding is lowered and, the observable half,
  that an exporter's file is never written). Green means: zero checker
  diagnostics anywhere (stricter than `run`'s cone gate on purpose — a
  warning outside the cone is still a wrong example), zero red, zero
  blocked; the failure names the example and the binding in `run`'s own
  words (`red \`xs\` — range: steps must be >= 1, got 0`, `blocked \`n\` —
  fed by red \`xs\``; verified by mutating `06-lists`). The wall IS
  included: measured cold in debug on the 24-core dev machine 6.9 s at
  cores − 2, 18 s at 4 threads, 34 s at 2 — the whole test runs in 7.8 s
  here; CI's 4-vCPU runners will sit near a minute, and the test's header
  says where the exclusion list goes if that ever dominates. The runner's
  own contract is pinned both ways (a red binding, its blocked dependant
  and a diagnostic are each reported, never skipped; a computed target, a
  memo hit within the solve and an exporter left alone each pass), as is
  discovery (the nested `wall/wall.cic` and a floor of seven files). **The
  review's fix round (2026-08-21)**: the first version demanded `Computed`
  of every target, and a VALID pipeline with two nodes of one
  content-addressed key (`reverse` applied twice more to its own result:
  `d = reverse(c)` has `b`'s key and is answered from `b`'s entry) failed
  as "`d` did not compute: CacheHit" — a false FAIL `cicada run` never
  gave ("3 computed, 1 from cache"), and for same-wave twins one that
  depended on the thread count (serial: a hit; parallel: both compute).
  An intra-solve hit is now green like `run`'s, and the `reverse³` case
  pins it (its hit is deterministic: the twin sits two waves downstream).
  The header and `examples/README.md` now state the exact differences
  from `run` — two, not one: diagnostics anywhere, and the working
  directory, which the test does not enter (`set_current_dir` is
  process-global and its tests run concurrently) — hence the rule that
  relative paths in non-effectful nodes must not rely on the cwd (nothing
  in `examples/` does: `export_obj` never runs, the wall's scripts resolve
  `inputs/` against `__file__`). Named, not done: the script host could
  spawn its workers with the pipeline's directory as their cwd so scripts
  see one rule under `run`, `serve` and this test alike — a
  `cicada-script`/`cicada-server` change, out of this package.
- ~~**Renumber item 4's orbit example**~~: done — `examples/08-orbit.cic`
- **Stale catalog on the client after a scripts-change reload** — **done
  2026-08-20** (`wt/hardening`; `web/src/state/catalog.ts`,
  `web/src/protocol/catalog.ts`; nothing on the wire changed,
  `PROTOCOL_VERSION` unchanged). The app fetched `/api/catalog` once at
  start; search rows and port tooltips for script nodes went stale until
  a page reload. The server labels no snapshot as a catalog change — the
  watcher's `reason` is `"external file change"` whether the pipeline
  text or a `scripts/*.py` moved, `git revert` says `"git revert"` with
  or without a script, and `apply_text` answers with a (non-barrier)
  snapshot only when a script changed — so the client cannot key on the
  reason, and a label would be one more thing a reload path could
  forget. The rule shipped needs no label: **every `snapshot` re-reads
  the catalog** (`CatalogRefreshPolicy`, fed from the connection module
  like the git policy). That is the join's snapshot too — the first
  connect, and every reconnect, where the scripts may have changed while
  the socket was down, a staleness the one-shot fetch never saw — so the
  start-time fetch is gone: the socket's first snapshot reads. One read
  in flight; any number of snapshots that land meanwhile collapse into
  exactly one follow-up, so reads land in order and an older catalog
  never ends on top of a newer one; a failed read is a notice and the
  previous catalog stays. Cost: one ~100 KB GET per snapshot (text-only
  reloads included), rare by construction. `window.__cicada.catalog()`
  reports `{reads, busy, nodes}`. Tests: the feed (every snapshot kind
  reads, `hello`/`delta`/statuses/lease/notices do not), the
  sequencing (a burst mid-read = one follow-up; a failed read keeps the
  owed one; dispose), the store path with an injected fetch (URL + token
  header, the whole object swapped, failure keeps the old catalog). Real
  app (debug engine, headless Chromium, a scratch copy of `examples/`
  with `05-script-geometry` open): the join reads once (108 nodes,
  `pyramids` in); a script written into `scripts/` → the watcher's
  barrier → exactly one more read, 109 nodes, search-to-place lists
  `Zzz Probe`; a text-only edit → one more read, no duplicates. Open, for
  the server side if the per-reload GET ever matters: an additive
  `catalog_changed` field on `snapshot` would let the client skip
  text-only barriers. **The review's fix round (2026-08-21)**: (1) every
  snapshot still reads, but an answer byte-identical to the one the store
  holds is no longer re-applied (`readCatalog` compares the response
  text, and trusts the comparison only while the store still holds the
  object it applied) — the store used to swap the catalog object on every
  read, and every canvas node subscribes to it, so a text-only reload
  re-rendered the whole canvas for a catalog that had not changed
  (`CicadaNode.tsx`'s "fires once per load" comment was stale and is
  corrected); `catalog.test.ts` pins that three identical answers leave
  one object in the store and a changed answer swaps. (2) The WIRING is
  now unit-tested: the review deleted `feedCatalogPolicy` from
  `startConnection`'s `onMessage` and 242 tests stayed green (only `tsc`
  noticed an unused import; a conditional mis-wire would have passed
  that too), the Playwright search spec being the only net.
  `connection.test.ts` drives `startConnection` against a fake
  `WebSocket` and a stubbed `fetch`: `hello` → no read, the join's
  snapshot → exactly one `GET /api/catalog?pipeline=…` with the token
  header and the answer in the store, a `delta` → none, a barrier → a
  second read, and `window.__cicada.catalog()` reports `{reads: 2, busy:
  false, nodes: 3}`; both mutations (feed removed; feed on `delta`
  instead) fail it. The Playwright suite was re-run on the branch's debug
  engine as part of this round (see the commit).
- **The costed, cancellable display edge** (WP-B review closure,
  2026-08-21): display tessellation now runs on the solve loop's workers
  before the session lock, tiered (preview / fine) and in parallel over
  a generation's distinct solids (docs/12 §Display cache) — but ONE
  giant solid is one `BRepMesh` call (the 300-hole bar: ~0.7 s per
  preview tick even at the preview tier, two 6,300-vertex planar faces),
  and a superseded tick's tessellation runs to completion. docs/12 names
  the rest: display as a costed, persisted edge in the store (the cache
  key is already the one it would use), so it is cancelled like a node,
  survives a warm reopen without the kernel, and can be routed to the
  kernel worker below when the cost model predicts it long.
- **Mixed-age stores, again** (WP-B): `LOG_FORMAT` is 3 — an engine from
  before `StoredValue::Solid` refuses a store this engine wrote; serve
  scratch copies across worktrees, as AGENTS.md says.
- **02-solids' export tessellation sits in the slider's cone**: the
  example's `shell = tessellate(solid=carved, deflection=0.01)` is
  recomputed on every tick (34 ms — the cone's whole cost now that
  display is 5 ms); it exists for the `--node dump` exporter. Either the
  example marks it as export-time work (`#off` until export, or a
  coarser deflection) or the docs/15 slider criterion names the scratch
  variant without it. A one-line decision for Ben; the measurement of
  both is in §Item 3.
- **The doc-12 kernel worker** (WP-C measurement): Esc inside ONE
  long OCCT boolean waits for it (1,000 tools: 1.7 s to idle). Route
  kernel calls the cost model predicts above ~1 s to a cancellable
  subprocess that Esc kills; the op-local sharing model makes this
  cheap — the call's inputs are bytes already. Until then a long B-rep
  boolean holds the session for its duration; an `each()` chain of small
  booleans does not (p95 31 ms). The worker is also the only complete
  answer to MEMORY inside one call: `tessellate`'s budget (review
  closure, 2026-08-21) bounds the one node whose request could grow the
  mesher without limit, but a boolean or a STEP read of a pathological
  input still runs in-process; a worker with a memory limit is killed,
  the node goes red, the session lives.
- **The memo hit checks arity, not kinds** (review closure 2026-08-21):
  a memo entry whose outputs are the wrong KIND for the node is served
  if its key matches. The version discipline (now a test — the signature
  ledger) is what prevents it; the structural guard would be output
  kinds in the memo record (`LogRecord` + `LOG_FORMAT` bump) compared
  against the decl on a hit. Worth it if a second class of stale entries
  ever appears.
- **The signature ledger is per-crate**: `tests/signatures.tsv` covers
  the stdlib's registry; script nodes (`scripts/*.py`) key on their body
  hash and need none. If a second node crate ever exists, it gets its
  own ledger through the same test.

- **`import_step` re-reads its file every solve** (volatile by design):
  a content-keyed memo (hash the file's bytes, key the translation on
  that) would keep the honesty and drop the read from a slider's cone
  when a large import sits in it. Measure a real import first.
- **Three cicada-server test fixtures were touched by WP-C** (outside its
  file list, to keep the rename's one commit green): `session.rs`'s
  mesh-display story and the OBJ-export story use `mesh_box`,
  `viewmodel.rs` expects `Solid` for `box`. Whoever owns the server
  crate should glance at them; nothing behavioural changed.
- **`cicada-geom/src/occt/glue.hxx` is a second glue location** (WP-C):
  the fork keeps the binding patches and the first glue, cicada-geom the
  node set's. If Ben prefers one home, the cicada-geom header moves into
  the fork's `cicada.hxx` in one patch and the bridge's `include!`
  changes; the `cicada_geom` C++ namespace keeps the two from colliding
  meanwhile. A decision for the ledger, not a blocker.

## Gates that must not regress (re-measured at each geometry or scheduler landing)

From doc 15 §Stage-6 results: cold wall carve < 10 s (6.5 s; 3.7 s on
2026-08-20 after WP-C; 3.75 s on 2026-08-21), warm < 100 ms (0.13 ms);
cheap-cone slider p50 ≤ 16 ms / p95 ≤ 33 ms (0.5 / 1.4 ms on the mesh
tier; the OCCT 02-solids cone 5.2 / 9.1 ms server, 5.5 / 9.4 ms client
on 2026-08-21 with display tiered and off the lock — measured on a
scratch copy without the example's export `tessellate` node, which
alone is 34 ms per tick and keeps the committed example at 39.9 / 54.9
ms, §Follow-ups); Esc time-to-idle p95 < 250 ms (214
ms; a 4,000-element B-rep chain 31 ms; ONE 1,000-tool boolean 1.8 s —
the kernel worker follow-up); file edit → canvas < 500 ms (~100 ms);
wall output equivalence `overall NOISE` on Windows and Linux.
